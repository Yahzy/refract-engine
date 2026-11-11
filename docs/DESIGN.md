# Refract — Document de conception

> Dernière mise à jour : 2026-10-31.

## 1. Vision

Refract est un **moteur de jeu** écrit en Rust : **ultra simple, moderne et optimisé**. Il servira ensuite de base à une **plateforme** de jeux communautaires : une liste de gamemodes, et pour chacun des serveurs communautaires.

- Un workflow éprouvé (entités, tick serveur, maps éditées en VMF, modding Lua), sans limites héritées : quand une approche moderne est plus simple ou plus rapide, on la prend, **sans dénaturer le rendu des assets importés**.
- **Simple** : peu de concepts, peu de code. Pas un moteur fourre-tout ; chaque système doit mériter sa place.
- **Moteur en Rust**, **gameplay communautaire en LuaJIT** (gamemodes, mods, serveurs).
- **Multijoueur** dès la conception (serveur autoritaire), même si le réseau arrive plus tard.
- **Compatibilité par conversion** : un outil séparé convertit les formats existants (BSP, VMF, MDL, VTF, VMT…) et les formats courants (glTF, puis FBX) dans un **format natif**. Le moteur ne lit que ce format. Le contenu existant est lu depuis les jeux installés par le joueur ; on ne redistribue rien.

## 2. Structure du dépôt

```
refract/
├─ Cargo.toml          workspace
├─ engine/             le moteur : tout ce qui tourne en jeu
│  ├─ core/            refract-core      maths (glam), repère et unités, angles ✅
│  ├─ platform/        refract-platform  fenêtre, boucle, entrées (winit) ✅
│  ├─ render/          refract-render    rendu wgpu + shaders Slang (shaders/*.slang, shaders/lib/) ✅
│  ├─ client/          binaire `refract` : terrain de jeu à pied ; maps converties et scène de test en caméra libre ✅
│  ├─ asset/           refract-asset     format natif : définition, écriture, lecture (mmap) ✅
│  ├─ physics/         refract-physics   corps rigides : interface sûre sur Box3D (sous-module box3d/, couche C shim/) ✅
│  └─ world/           refract-world     collisions, entités, props physiques, mouvement du joueur, ticks fixes, sans fenêtre ni GPU ✅
│     plus tard : net/, script/ (LuaJIT), audio/, server/ (binaire dédié)
├─ tools/              ce qui prépare le contenu, hors du jeu
│  ├─ formats/         refract-formats   lecteurs de formats : VFS Steam/VPK/GMA, BSP, MDL, VTF, VMT, KeyValues
│  └─ convert/         refract-convert   BSP/VMT/VTF/MDL → format natif (glTF/FBX plus tard) ✅
├─ docs/
└─ game/               dossier de jeu de développement (cache, addons, mount.toml local…)
```

Les crates d'`engine/` sont créées au fur et à mesure de leur premier code, pas en avance.

### Règles de structure

- **`engine/*` ne dépend jamais de `tools/*`.** Toute la complexité des formats importés reste dans les outils (vérifié par le test `engine/core/tests/structure.rs`).
- **Dépendances dans un seul sens** : `core` → `asset` → `world` / `render` → `client`, et `physics` → `world`. `world` ne connaît ni la fenêtre ni le GPU (il doit pouvoir tourner dans un serveur dédié) ; `render` lit l'état du monde, il ne le modifie pas.
- **Une crate = une responsabilité**, un module = un système, des fichiers courts nommés d'après ce qu'ils font.
- Pas d'état global (seule exception : l'unité de longueur de la bibliothèque physique, fixée une fois pour le processus). `thiserror` dans les bibliothèques, `anyhow` dans les binaires.
- Code, commentaires et commits en anglais.

## 3. Décisions techniques

| Sujet | Choix | Raison |
|---|---|---|
| Langage | Rust (édition 2024) | |
| Rendu | **`wgpu`**, backend **Vulkan** | Contrôle total du rendu et de son coût CPU ; DX12 testé à 14 FPS |
| Shaders | **Slang**, compilé en SPIR-V au build (`build.rs` + `slangc` du Vulkan SDK), donné tel quel à Vulkan (`PASSTHROUGH_SHADERS`, sans naga) | Langage moderne (modules, génériques, interfaces) ; remplace WGSL + naga_oil et leurs pièges |
| Fenêtre / entrées | `winit` | |
| Repère | **Z vers le haut**, X devant, Y à gauche, dans tout le moteur | Le repère des maps importées et du Lua des moddeurs |
| Unités | **1 unité = 1 pouce** | Pas de conversion des maps importées, valeurs familières aux moddeurs ; la précision relative des flottants ne dépend pas de l'échelle. Les imports glTF/FBX (en mètres) sont mis à l'échelle par le convertisseur, la physique convertit à sa frontière si besoin |
| Scripting | `mlua` + **LuaJIT**, sandboxé | Rapide et connu des moddeurs ; le code des serveurs s'exécute chez les joueurs |
| Maths | `glam` | Standard de l'écosystème |
| Collisions du joueur | **maison** : brushes convexes, balayage d'une boîte | Le modèle qui donne marches, glisse sur les pentes et surf exactement ; c'est aussi la forme des volumes des maps converties |
| Physique des objets | **Box3D** (C, licence MIT), sous-module git `engine/physics/box3d` épinglé sur un commit, compilé par `build.rs` avec `cc` | Moteur récent, rapide (SIMD, sommeil, continu), API C simple. Encore en alpha : on monte de version quand on veut. Une couche C maison (`shim/physics.c`) ne fait passer que des valeurs simples, pour ne pas recopier en Rust ses grosses structures de définition ; un test compare les tailles des structures partagées |
| Réseau | à choisir (UDP, snapshots + delta) | Plus tard |
| Formats d'import | BSP, VMF, MDL, VTF, VMT, **glTF** d'abord, FBX ensuite (`ufbx`) | glTF est le standard ouvert ; FBX est propriétaire et complexe |

## 4. Chaîne de contenu

```
Steam (VPK, BSP, MDL…) ─┐
glTF / FBX ─────────────┼─► refract-convert ─► format natif ─► moteur (lecture mmap)
VMF ────────────────────┘        ▲
                       rechargement en direct à chaque sauvegarde du .vmf
```

- **Conversion à la demande, avec cache** : `+map x` lance `refract-convert map x`, qui convertit la map et seulement les matériaux, textures et modèles qu'elle utilise, dans `game/cache/content/` (chemins calqués sur ceux du contenu : `materials/a/b.rmat`, `textures/a/b.ktx2`, `models/a/b.rmdl`, `maps/x.rmap`). Une map est à jour tant que l'empreinte de son BSP et la version du convertisseur (`CONVERTER_VERSION`, à incrémenter quand sa sortie change) ne bougent pas : la vérification est instantanée. Les assets déjà convertis servent à toutes les maps.
- **Format natif** (`refract-asset`) :
  - **conteneur binaire** des maps et modèles : en-tête `RFCT` (type, version), table de sections, sections alignées sur 16 octets, lues en place depuis un mmap (tableaux de structures, ou texte) ;
  - **géométrie** : deux flux de sommets (positions seules pour les passes de profondeur et d'ombre ; attributs de 24 octets : normale octaédrique, tangente + signe, UV, couleur dont l'alpha porte le mélange des deux couches d'un displacement), indices 32 bits, sous-maillages (plage d'indices, matériau, drapeaux, sphère englobante), liste de matériaux ;
  - **modèle** `.rmdl` : une géométrie ; un fichier par skin (`.skin2.rmdl`) ;
  - **map** `.rmap` : `INFO` en TOML (soleil, ambiant plat, caméra de la skybox 3D, brouillard…), géométrie du monde en blocs (drapeaux : skybox 3D, overlay, ombre seulement), props (`PROP` + liste de modèles), lumières (`LITE`, avec la marge de leur luminaire, voir plus bas), grille de sondes de lumière (`PRBG` + `PROB` : par sonde 6 couleurs RGB9E5, 6 distances libres en f16, un drapeau « valide »), entités en texte (`clé<TAB>valeur`, une ligne vide entre deux entités) ;
  - **matériau** `.rmat` en TOML : `kind` (opaque, alpha_test, translucent, additive, water, sky, invisible), texture de base et normal map (vert vers +V), teinte, albédo moyen (pour le baker), rugosité, métal, seuil alpha, double face, seconde couche (couleur, normal map, texture de modulation du mélange), texture de détail (échelle, mode : `modulate2x`, `add`, `over`, `under`, `multiply`, `glow`, force, teinte), défilement, paramètres de l'eau (table `water`, voir §5). Les `$ssbump` sont convertis en normal maps ; les détails `modulate2x` sont stockés en linéaire (facteurs autour du gris moyen) ;
  - **texture** `.ktx2` (standard Khronos) : formats GPU, mips, couches (frames d'animation), faces de cubemap. Les VTF DXT sont copiés tels quels (sans perte ni réencodage), les autres encodés en BC7 (`intel_tex_2`).
- **Conversion des lumières** : les intensités des lumières (`_light`) sont mises à l'échelle pour qu'une surface blanche éclairée à 1 apparaisse blanche. Le pitch d'une entité lumière est positif vers le haut (contrairement aux angles de vue), qu'il vienne de `pitch` ou d'`angles` ; `angle` -1/-2 = haut/bas.
- **Luminaires** : les maps placent leurs lumières dans des modèles (globes, abat-jour, boîtiers). Pour chaque lumière, le convertisseur cherche un petit prop (192 unités au plus) dont la boîte la contient ; la lumière reçoit une **marge** (au moins 32 unités, sinon jusqu'au coin le plus éloigné du luminaire) que les rayons d'ombre ignorent, au rendu comme dans le bake.
- **Lumière indirecte : baker GPU** (`refract-render`, module `gi`, appelé par le convertisseur) :
  - une grille de sondes sur le monde jouable (jusqu'à 2 M sondes, au moins 64 unités entre deux) ;
  - lancer de rayons matériel (ray query de wgpu, expérimental, Vulkan) contre une structure d'accélération du monde, des brushes cachés et des props opaques, chaque triangle avec l'albédo moyen de son matériau ; la skybox 3D y est placée comme le client la dessine (agrandie autour de la caméra de la skybox), pour que reflets et lumière du ciel la voient ;
  - par sonde, 64 rayons (Fibonacci sphérique, tournés à chaque passe) ; à l'impact : soleil et lumières proches (triées par une grille de cellules de 512 unités) avec rayons d'ombre, plus la lumière des sondes de la passe précédente ; sans impact : le ciel. 3 passes = 3 rebonds ;
  - par sonde : un ambient cube, la distance libre moyenne dans les 6 directions, et « valide » si moins d'un quart des rayons touchent des faces arrière (sinon la sonde est dans un mur). Les surfaces fines (double face ou alpha test : feuillages, grillages) sont éclairées des deux côtés et leur dos ne compte pas comme une face arrière, sinon les sondes dans les arbres seraient « dans un mur » ;
  - les sondes dans les murs reçoivent ensuite la lumière de leurs voisines valides (3 passes de dilatation), tout en restant marquées non valides ;
  - petite map : 2 M sondes en 0,7 s ; grande map de ville : 2 M sondes sur 3,7 M triangles en 1,7 s (RTX 4070 Ti SUPER) ;
  - sans GPU capable de lancer des rayons, la même grille est remplie avec les ambient cubes du BSP (sans visibilité, cellules vides comblées par leurs voisines) ;
  - le dimensionnement de la grille, la dilatation et l'empaquetage des cubemaps (`gi::probes`) servent aussi au client, qui cuit les niveaux construits en code au lancement.
- **Ciel** : converti en cubemap HDR (`Rgba16Float`, 256² par face) aux conventions du moteur (échantillonnée avec une direction du monde) : le ciel peint (`skyname painted` / `env_skypaint`, sa formule évaluée pour chaque texel, couche d'étoiles ou de nuages au repos) ou les six textures de la skybox rééchantillonnées. Affiché à 1,5×. Pour le bake, la lumière du ciel est résumée en dégradé zénith / horizon / sol : chacun est la lumière que reçoit une surface tournée dans ce sens (moyenne pondérée par le cosinus sur la moitié du ciel qu'elle voit), avec la **couleur de l'ambiant de la map** (`_ambient`) ramenée à mi-chemin du gris : les textures de ciel sont bien trop bleues pour éclairer et donnaient des ombres bleu marine, et l'ambiant seul les laisse encore nettement bleues là où seul le ciel éclaire. Hors des sondes, le moteur prend 40 % du texel du ciel vu selon la normale.
- **Reflets (cubemaps)** : aux positions des `env_cubemap` de la map, le baker trace aussi des cubemaps (64² par face, un rayon par texel) : un impact est éclairé comme dans le bake des sondes (soleil, lumières, sondes finies) avec l'albédo moyen de son matériau ; sans impact, la cubemap du ciel. Stockées en tableau de cubemaps KTX2 `Rgba16Float` avec tous leurs mips (filtre 2×2). Les matériaux à `$envmap` deviennent lisses (rugosité 0,2) et plus réfléchissants (`reflectance`, F0 selon `$envmaptint`) ; seuls eux reflètent une cubemap, les autres gardent le spéculaire des sondes. Chaque face du monde reflète la cubemap que la map lui a attribuée (son matériau dérivé nomme `maps/<map>/c<x>_<y>_<z>`), les props la plus proche. Le niveau de mip suit la rugosité.
- **Rendu des sondes** : storage buffer lu par le shader ; pour chaque pixel, les 8 sondes voisines, lues une seule fois pour le diffus (selon la normale) et le spéculaire (selon la réflexion), pondérées par la trilinéaire, la validité (une sonde dans un mur ne compte presque pas : seulement quand aucune sonde valide n'est proche, interstices et petites pièces), l'orientation (sonde derrière la surface) et la visibilité (la sonde voit-elle jusqu'à la surface ? coupure progressive sur une demi-cellule, une coupure nette dessinait des cercles), ce qui évite les fuites de lumière à travers les murs. Hors de la grille (skybox 3D), le ciel. Un ambiant plat gris (10 % de la luminance de l'`_ambient`) s'y ajoute partout. La même grille pourra plus tard être mise à jour en temps réel (façon DDGI).
- Les maps restent éditées en **VMF** dans un éditeur externe : le moteur surveille le `.vmf`, le reconvertit et recharge la map en une seconde environ.
- **Éclairage** des maps issues d'un VMF : baker GPU maison (pas de lightmaps précalculées par un compilateur externe), avec une architecture ouverte à l'éclairage global temps réel.

### Montage du contenu (dans `refract-formats`)

- Steam est détecté via le registre et `libraryfolders.vdf`.
- Ordre de recherche : le dossier de jeu local (`game/`), puis les addons et les éléments du Workshop des jeux montés (assets seulement), puis le contenu de ces jeux (archives VPK et dossiers), dans l'ordre de `game/mount.toml`. Ce fichier, propre à chaque machine, n'est pas versionné :

```toml
[[game]]
steam = "Some Game"                         # dossier dans steamapps/common
content = ["base/pak01_dir.vpk", "base"]    # archives VPK et dossiers, dans l'ordre de recherche
addons = "base/addons"                      # dossiers d'addons et archives .gma
workshop = 123456                           # app Steam dont les éléments du Workshop sont montés
```

- Chemins virtuels en minuscules, séparés par `/` : `models/props/barrel01.mdl`, `materials/...`, `maps/x.bsp`.

## 5. Moteur

### Monde et préparation du multijoueur

Le réseau n'est pas codé tout de suite, mais tout est préparé pour lui :

- **simulation à tick fixe** dans `refract-world`, indépendante de la cadence d'affichage ;
- **état du monde séparé du rendu** : le rendu lit un état (interpolé entre deux ticks), il ne le modifie jamais ;
- **`world` sans fenêtre ni GPU**, pour qu'un serveur dédié headless soit un simple autre binaire ;
- **identifiants d'entités stables** et sérialisables, pour les snapshots réseau ;
- **serveur autoritaire** : le client enverra ses commandes, prédira son propre mouvement, et le serveur tranchera.

### Joueur et collisions (`refract-world`)

- **Collisions** : le monde solide est une liste de **brushes** convexes (intersections de demi-espaces). Le joueur est une boîte alignée sur les axes (32 × 32 × 72, 36 de haut accroupi), dont l'origine est le centre du dessous. Balayer une boîte contre un brush revient à balayer un point contre le brush grossi par la boîte, encore borné par des plans : ceux du brush repoussés du coin de la boîte le plus enfoncé, plus des plans **biseaux** (les directions des faces de la boîte, et les arêtes de la boîte croisées avec celles du brush, là où ce plan ne fait que toucher le brush), calculés une fois à la création du brush. Un balayage découpe donc un segment par une poignée de plans, s'arrête à 1/32 d'unité avant la surface, et dit s'il part ou reste dans un solide. Les boîtes englobantes écartent les brushes loin du trajet (pas encore de structure spatiale : inutile pour quelques centaines de brushes).
- **Mouvement** (`Player::step`, un tick par commande) : la commande donne les angles de vue, les axes avant/côté/haut et les boutons (saut, accroupi, course, marche lente). Dans l'ordre : accroupi ; demi-gravité ; saut (seulement sur un nouvel appui, sauf `auto_jump` ; s'ajoute à la vitesse verticale, la remplace accroupi, et termine aussitôt la gravité de ce tick) ; au sol, friction puis accélération et déplacement ; en l'air, accélération aérienne et glissement ; état du sol ; seconde demi-gravité (intégration exacte d'une accélération constante).
  - **sol** : friction proportionnelle à la vitesse (au moins `stop_speed`), accélération vers la vitesse voulue ; le déplacement essaie le trajet droit, sinon à la fois le glissement à plat et le même glissement surélevé d'une marche (18 unités) puis redescendu, et garde le plus long ; ensuite il colle au sol qui descend de moins d'une marche (escaliers descendus sans décoller) ;
  - **air** : l'accélération ajoute de la vitesse le long de la direction voulue jusqu'à 30 seulement (avec le taux de la vitesse voulue entière) : on dirige sans propulser, mais en tournant pendant un strafe chaque tick trouve une nouvelle direction où ajouter 30, d'où l'**air strafe** et le **bunny hop** (sauter au tick d'atterrissage saute la friction) ;
  - **glissement** : jusqu'à 4 rebonds ; la vitesse perd sa part dans chaque surface touchée, glisse le long du pli entre deux surfaces, s'arrête dans un coin ; une pente trop raide (normale à moins de 0,7 de hauteur, environ 45°) n'est pas du sol : on y glisse comme en l'air, ce qui donne le **surf**. Un balayage arrêté avant d'avoir bougé (l'arrondi a laissé la boîte plus près de la surface que les 1/32 d'unité gardés) écarte la boîte de ces 1/32 le long de la normale : sinon chaque balayage suivant le long de la surface s'arrête aussi, et après 4 la vitesse tombe à zéro (le joueur restait collé aux rampes de surf) ;
  - **sol ou pas** : une boîte balayée de 2 unités vers le bas ; monter à plus de 140 unités/s décolle ; poussé vers le haut contre une pente trop raide, l'accélération et le contrôle en l'air tombent à 25 % ;
  - **accroupi** : au sol, la vue descend en 0,2 s puis la boîte rétrécit par le haut ; en l'air, la boîte rétrécit aussitôt par le bas (les pieds remontent de 36 : le saut accroupi monte plus haut) ; se relever demande de la place ;
  - **crouch** au sol seulement : la vitesse réduite ne s'applique pas en l'air ;
  - **noclip** : vol à travers tout, à vitesse fixe selon la vue (500, ×3 en courant, ×0,1 accroupi, Espace pour monter), sans accélération ni friction : relâcher arrête net ; la vitesse n'est pas remise à zéro en entrant ou en sortant du noclip, on repart donc avec l'élan du vol.
- **Valeurs** (`MoveSettings`, ajustables) : gravité 600, friction 8, `stop_speed` 10, accélération 10 au sol comme en l'air, 30 en l'air, marche 200, course 400, marche lente 100, accroupi ×0,3, saut à 200 unités/s (33 unités de haut), marche 18, vue à 64 (28 accroupi).
- **Ticks** : 66,67 par seconde (`TickClock` accumule le temps des images, rattrape au plus 0,25 s). Le client interpole la position des yeux entre les deux derniers ticks, et lisse les marches (la vue rattrape la hauteur en ~0,06 s au lieu de sauter de 18 unités).
- **Client** : la souris tourne la vue à chaque image (0,066° par compte), les touches sont lues à chaque tick ; un appui sur Espace ou un cran de molette attend le tick suivant (sauter à la molette). Échap libère la souris, puis quitte ; un clic la reprend.

### Entités et props physiques (`refract-world`, `refract-physics`)

- **Entités** : `Entities<T>` range des valeurs sous des `EntityId` (index + génération) ; un id retiré ne désigne jamais la valeur qui reprend son emplacement. L'id tient sur 64 bits (`to_bits`), pour le réseau et les scripts.
- **`refract-physics`** : interface sûre sur Box3D (mondes, corps statiques, cinématiques ou dynamiques, formes boîte, sphère et enveloppe convexe, impulsions, requêtes par boîte). Les ids de corps sont vérifiés à chaque appel (panique après destruction, jamais de comportement indéfini). La bibliothèque travaille dans les unités du moteur : on lui donne le nombre d'unités par mètre, donc positions, vitesses et gravité sont en pouces ; seules les densités restent en kg/m³ (converties à la frontière) et les masses en kg.
- **`World`** : les brushes du niveau (collisions du joueur, et un corps statique fait d'une enveloppe convexe par brush pour la physique) et les **props** (une entité, un corps dynamique, une forme boîte ou sphère). La physique avance au même tick que le joueur (4 sous-pas) ; chaque prop garde sa pose du tick précédent, et le client dessine entre les deux. Une prop tombée sous -16384 est retirée.
- **Joueur et props** : le joueur garde son propre mouvement. Avant chaque tick, les props proches (requête par boîte autour du déplacement) deviennent des brushes à leur pose actuelle (plans des faces tournés, biseaux recalculés : ils suivent les axes du monde) et passent dans les mêmes balayages que le niveau (trait `Solid`). On marche donc sur une caisse, on la monte comme une marche, on s'y cogne. Une sphère est vue par le joueur comme un solide à 26 faces qui l'entoure. Les lancers de formes de la bibliothèque ne servent pas ici : ils signalent un contact initial dès 0,2 unité de distance, ce qui bloquerait un joueur posé sur une caisse.
  - **pousser** : chaque prop heurtée de côté reçoit l'impulsion d'un choc mou entre le joueur (85 kg) et elle, limitée au poids du joueur sur le tick ; le joueur garde ensuite la vitesse que la prop a prise. Une caisse de 10 kg suit la marche, une de 100 kg avance lentement, une de 1000 kg ne bouge pas ;
  - **peser** : la prop sous les pieds reçoit le poids du joueur à chaque tick (une bascule penche) ;
  - **recevoir** : un corps cinématique en boîte suit le joueur (vitesse fixée pour rejoindre sa position, un peu plus petit que sa boîte pour ne pas frotter ce qu'il longe), si bien que les props tombent sur le joueur et y restent posées au lieu de le traverser ; il disparaît en noclip et est refait quand le joueur s'accroupit ou se relève.
- **Client** (`game.rs`, `props.rs`) : `Game` lit les commandes, fait tourner les ticks (joueur puis physique) et place une instance du rendu par prop (maillage partagé par forme et taille, instances cachées réutilisées). F lance une caisse, G une balle, H une grande caisse légère, Retour arrière retire ce qui a été lancé. `--screenshot` laisse la physique tourner `--time` secondes, joueur immobile.

### Terrain de jeu (`engine/client/src/playground.rs`)

Le niveau chargé sans `+map` : construit en code avec des brushes, qui donnent à la fois les collisions, la géométrie dessinée (textures projetées selon l'axe principal de chaque face, 256 unités par répétition) et ce que trace le baker. Textures générées en code : une grille (cellules de 16, dalles de 64) et le relief des dalles en normal map. Au lancement, le baker GPU cuit les sondes (cellules de 32 unités, ~180 000 sondes, 3 rebonds) et 6 cubemaps de reflet, en 0,2 à 0,3 s ; sans GPU capable de lancer des rayons, le ciel seul éclaire. Zones : escaliers, marches trop hautes, rampes de 30° et 55°, caisses de 32 à 80 de haut ; rampe de surf à 60° sous une plateforme à 384 ; plateformes à sauter au-dessus d'un sol lumineux ; hall couvert aux lumières colorées avec tunnel à franchir accroupi ; vitrine de matériaux (sphères de rugosité et métal, piliers d'acier, d'or et de cuivre, sol noir brillant, lumières en orbite avec ombres) ; devant le départ, de la physique : pyramide de caisses de 20 kg, tour de petites caisses, balles de tailles croissantes, caisse d'acier de 800 kg trop lourde à pousser, bascule (planche sur un coin).

### Rendu (`refract-render`)

Objectif : **parité visuelle** avec l'ancien visualiseur, vérifiée par captures comparées. Ce qui existe :

- **Ressources persistantes** : maillages, matériaux et instances sont envoyés une fois au GPU. Tous les maillages partagent un seul vertex buffer et un seul index buffer (aucun changement de buffer entre deux draws). Ce qui change à chaque image (caméra, lumières, ciel, soleil) passe dans une `Frame`.
- **Passes** : ombres → scène (ciel procédural puis géométrie opaque, forward) dans une cible HDR `Rgba16Float` → tonemapping (exposition + courbe ACES ajustée) vers la sortie sRGB.
- **Profondeur inversée** (1 au plan proche, 0 à l'infini, projection sans plan lointain) : précision uniforme sur toute la distance de vue.
- **PBR métal-rugosité** : GGX, visibilité de Smith corrélée, Fresnel de Schlick, diffus de Lambert. Lumière ambiante du ciel : diffuse selon la normale, spéculaire selon la réflexion (approximation analytique de la BRDF d'environnement), en attendant l'IBL des cubemaps.
- **Lumières dynamiques** : soleil, lumières ponctuelles et spots, en unités relatives (une lumière ponctuelle d'intensité 1 éclaire à 1 mètre comme un soleil d'intensité 1), atténuation en carré inverse fenêtrée jusqu'à `range`. Toutes les lumières sont bouclées dans le shader : à remplacer par un découpage en clusters quand il y en aura beaucoup.
- **Ombres dynamiques** :
  - soleil : 4 cascades de 2048², réparties entre logarithmique et uniforme jusqu'à 6000 unités. Chaque cascade est dimensionnée par la sphère exacte de sa tranche (rayon indépendant de l'orientation de la caméra) et calée sur les texels : pas de scintillement quand la caméra bouge ou tourne ;
  - lumières locales : un tableau partagé de 16 couches de 1024² ; un spot prend une couche, une lumière ponctuelle six (faces de cube à 92° pour filtrer sans couture). Les lumières visibles les plus proches de la caméra sont servies en premier, les autres n'ont pas d'ombre ;
  - filtrage PCF (9 comparaisons bilinéaires, filtre en tente 4×4), biais de pente au rendu et décalage selon la normale à la lecture ;
  - une instance peut ne pas projeter d'ombre (`set_casts_shadows`), par exemple l'ampoule à l'intérieur d'une lumière.
- **Culling** : chaque instance a une sphère englobante ; la vue et chaque carte d'ombre ne dessinent que ce que leur frustum contient (les plans à l'infini sont ignorés).
- **Captures et mesures** : `--screenshot` rend hors écran et relit l'image ; `--bench <frames>` donne le temps CPU (préparation + soumission) et le temps total par image.
- **Maps** : la géométrie d'une map (ou d'un modèle) est envoyée en une fois depuis le mmap ; chaque bloc ou sous-maillage devient une instance. Textures **bindless** (un tableau de toutes les textures, indexé par matériau ; groupe de bind séparé, wgpu ne mélangeant pas tableaux et uniform buffers), échantillonnage anisotrope 16×. Matériaux opaques, alpha test, double face, eau, transparents et additifs (cinq pipelines, dessinés dans cet ordre ; transparents pas encore triés). Lumière indirecte : les sondes cuites (voir §4). Champ de vision vertical 75°. Tonemapping **Khronos PBR Neutral**, qui garde les couleurs des textures telles qu'elles ont été peintes.
- **Lumières en clusters** : la vue est découpée en 16 × 9 tuiles × 24 tranches de profondeur (exponentielles jusqu'à 16384 unités) ; un compute shader liste dans chaque cluster les lumières dont la portée l'atteint (128 au plus), et chaque pixel ne boucle que sur celles de son cluster. Le CPU n'envoie que les lumières visibles (jusqu'à 4096). Les lumières qui demandent des ombres se partagent le tableau d'ombres locales ; **toutes les autres ont des ombres par rayons** quand le GPU lance des rayons en matériel : pour chaque lumière qui touche un pixel, un rayon d'ombre (ray query) contre les surfaces opaques de la scène, en ignorant la marge du luminaire de la lumière (voir §4). Le moteur construit lui-même ces structures d'accélération (une par maillage, une qui place les instances opaques projetant des ombres, reconstruite quand elles changent). Sans ce matériel, ou avec `-noraytracing`, le shader est compilé sans cette variante (`// variant:` dans `scene.slang`, géré par `build.rs`) et les lumières de map traversent les murs. Coût sur une grande map de ville : de 0,2 à 1 ms par image.
- **Passe de profondeur préalable** : les surfaces opaques écrivent d'abord leur profondeur (les surfaces en alpha test à travers leur texture), puis sont ombrées avec un test « égal » : chaque pixel n'est ombré qu'une fois. Les deux passes transforment les positions par les mêmes fonctions de shader, donc trouvent exactement les mêmes profondeurs.
- **Ombres des surfaces en alpha test** : un pipeline d'ombre à part lit leur texture et ne garde que les texels opaques (feuillages, grillages).

Mesures (scène de départ, 1280×720, `--bench`) : petite map 0,3 ms de CPU et ≈ 1,2 ms par image ; grande map de ville 1,6 ms de CPU et ≈ 4,3 ms par image avec ses 1027 lumières. Conversion complète de la grande map : 6 s la première fois, ≈ 2 s pour la map seule (dont 1,7 s de bake) ; chargement 1,5 s initialisation du GPU comprise.

- **Surfaces** : normal maps (repère tangent par sommet, la normale géométrique place les lectures d'ombre et pondère les sondes, la normale de la texture éclaire), seconde couche des displacements (mélange par l'alpha des sommets, affiné par la texture de modulation), textures de détail.
- **Brouillard** : brouillard de hauteur exponentiel, calculé en forme close dans le shader de la scène et celui du fond (quelques instructions par pixel, ni marche dans un volume ni passe supplémentaire). Sa densité décroît avec l'altitude (d'un facteur e tous les 4096 unités au-dessus du point d'apparition), ce qui voile l'horizon et laisse le zénith dégagé ; sa lumière est sa couleur (le ciel) plus 30 % de soleil diffusé vers l'avant (Henyey-Greenstein, g = 0,6), qui fait briller la brume autour du soleil. Le convertisseur traduit le brouillard linéaire de la map (`env_fog_controller` : début, fin, densité max, couleur affichée à 1,5× comme le ciel) : le brouillard est à moitié opaque là où l'ancien était à mi-chemin, puis s'épaissit doucement au lieu de buter sur un mur à la distance de fin. Le décor lointain (skybox 3D, instances marquées `set_distant`) et le ciel prennent la densité du brouillard de la skybox (`sky_camera`, distances multipliées par son échelle). Les surfaces additives ne font que s'estomper.

- **Eau** (matériaux `water`) : après les surfaces opaques, l'image HDR et la profondeur sont copiées (seulement si de l'eau est visible), puis une seconde passe de rendu dessine l'eau (pipeline à part, groupe de bind 2 : ces copies et les paramètres de l'eau), puis les transparents et les additifs. L'eau écrit sa profondeur : ce qui est dessous et transparent est caché, comme le reste de ce qu'elle recouvre. Dans le shader :
  - **vaguelettes** : la normal map (`$normalmap`), défilant selon le proxy `TextureScroll` de `$bumptransform`, ses frames animées par `AnimatedTexture` ; avec `$scroll1`, deux couches de plus, tournées et remises à l'échelle, défilant à `$scroll1` et `$scroll2` ;
  - **ce qu'on voit à travers** : la copie de l'image, décalée par les vaguelettes (`$refractamount`), sauf si le décalage tombe sur un objet devant l'eau ; teintée par `$refracttint` puis noyée dans la couleur des profondeurs (`$fogcolor`) selon l'épaisseur d'eau traversée jusqu'au fond (`$fogstart`, `$fogend`). Cette couleur est éclairée comme une surface mate tournée vers le haut (sondes et soleil avec son ombre), à moitié (l'eau absorbe le reste) : de nuit ou à l'intérieur, l'eau s'assombrit ;
  - **reflet** : tracé sur la copie de l'image (48 pas sur l'écran le long du reflet de la surface plane, affinés par dichotomie, 1/profondeur interpolée linéairement), décalé par les vaguelettes (`$reflectamount`), teinté par `$reflecttint` ; là où l'image ne contient pas ce qui est reflété (hors de l'écran, derrière un objet), la cubemap de reflet de la surface, sinon le ciel. Pas de second rendu de la scène en miroir : l'eau ne coûte qu'une copie et un shader (≈ 0,2 ms quand un lac occupe un quart de l'écran) ;
  - mélange par Fresnel (F0 = 0,02), qui s'estompe sur les 16 premières unités de profondeur pour adoucir les rives ; reflets du soleil (GGX, rugosité 0,08) avec son ombre ; brouillard de l'air par-dessus ;
  - **vue de dessous** : la face inférieure générée par le compilateur de map (`$abovewater 0`, matériau `$bottommaterial`) montre ce qu'il y a au-dessus, noyé selon la distance à l'œil, sans reflet.

À venir pour la parité : défilement de la couche de nuages du ciel peint, masques des reflets (`$envmapmask`…) et textures dans les reflets, brouillard d'eau quand la caméra est immergée, sprites, proxies de matériaux, distance d'affichage des props, ordre de profondeur de la skybox 3D.

## 6. Feuille de route

1. **Structure** : `engine/` + `tools/`. ✅
2. **Nouveau rendu**, première étape : wgpu + Slang, PBR, lumières et ombres dynamiques, sur une scène de test. ✅
3. **Format natif** (`refract-asset`). ✅
4. **`refract-convert`** : BSP → format natif ; le client charge les maps converties. ✅
5. **Nouveau rendu** jusqu'à la parité avec l'ancien visualiseur par captures (liste en §5).
6. **Import VMF** + rechargement en direct, puis **baker d'éclairage GPU**.
7. **Couche jeu** : `world` (entités, tick), physique, LuaJIT, puis réseau et serveur dédié. Commencée le 2026-10-31, rendu mis en pause : joueur au mouvement complet sur le terrain de jeu ; entités et props physiques (Box3D) le 2026-11-06.
8. **Plateforme** (section 7).

## 7. Pistes pour la couche jeu et la plateforme

À rediscuter au moment de l'étape 7, rien n'est décidé.

- **Modding Lua** : hooks, messages réseau, entités et armes scriptées, gamemodes ; une API familière aux moddeurs, sur une base propre.
- **Côtés** serveur / client dans un même code Lua (fichiers serveur, client et partagés).
- **Entités** : index numérique, `classname`, classes de base (entité de base, prop physique, joueur, monde…), `Think`, `Initialize`, `OnRemove`, `Use`, `Touch` ; entités natives en Rust, entités scriptées en Lua.
- **Network vars** répliquées par snapshots + delta génériques (pas de SendTable/RecvTable) ; bibliothèque `net` pour les messages Lua.
- **Commandes du joueur + mouvement** prédit côté client ; variables et commandes de console.
- **Sécurité** : Lua client sandboxé (pas de `io`, `os.execute`, `require` natif ni `debug`, `file.*` limité à `data/`) ; seuls les fichiers marqués client sont envoyés ; messages `net` limités en taille et en débit.
- **Plateforme** : pas de solo ni de menu sandbox local ; on choisit un gamemode, puis un serveur. Master server en Rust (heartbeat, liste par gamemode, joueurs, map, ping), téléchargement de contenu à la connexion avec cache, authentification Steam ou comptes maison.

## 8. Points à trancher plus tard

- Transport réseau
- Attraper et lancer les props (outil façon pistolet à gravité), contraintes et joints, props aux formes des modèles convertis
- Authentification (Steam ou comptes maison), hébergement des addons
