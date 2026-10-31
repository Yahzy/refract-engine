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
│  ├─ client/          binaire `refract` : maps converties ou scène de test, caméra libre ✅
│  ├─ asset/           refract-asset     format natif : définition, écriture, lecture (mmap) ✅
│  └─ world/           refract-world     entités + simulation à tick fixe, sans fenêtre ni GPU
│     plus tard : physics/, net/, script/ (LuaJIT), audio/, server/ (binaire dédié)
├─ tools/              ce qui prépare le contenu, hors du jeu
│  ├─ formats/         refract-formats   lecteurs de formats : VFS Steam/VPK/GMA, BSP, MDL, VTF, VMT, KeyValues
│  └─ convert/         refract-convert   BSP/VMT/VTF/MDL → format natif (glTF/FBX plus tard) ✅
├─ docs/
└─ game/               dossier de jeu de développement (cache, addons, mount.toml local…)
```

Les crates d'`engine/` sont créées au fur et à mesure de leur premier code, pas en avance.

### Règles de structure

- **`engine/*` ne dépend jamais de `tools/*`.** Toute la complexité des formats importés reste dans les outils (vérifié par le test `engine/core/tests/structure.rs`).
- **Dépendances dans un seul sens** : `core` → `asset` → `world` / `render` → `client`. `world` ne connaît ni la fenêtre ni le GPU (il doit pouvoir tourner dans un serveur dédié) ; `render` lit l'état du monde, il ne le modifie pas.
- **Une crate = une responsabilité**, un module = un système, des fichiers courts nommés d'après ce qu'ils font.
- Pas d'état global. `thiserror` dans les bibliothèques, `anyhow` dans les binaires.
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
| Physique | à choisir (`rapier3d`, Jolt…) | Plus tard |
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
  - **matériau** `.rmat` en TOML : `kind` (opaque, alpha_test, translucent, additive, water, sky, invisible), texture de base et normal map (vert vers +V), teinte, albédo moyen (pour le baker), rugosité, métal, seuil alpha, double face, seconde couche (couleur, normal map, texture de modulation du mélange), texture de détail (échelle, mode : `modulate2x`, `add`, `over`, `under`, `multiply`, `glow`, force, teinte), défilement. Les `$ssbump` sont convertis en normal maps ; les détails `modulate2x` sont stockés en linéaire (facteurs autour du gris moyen) ;
  - **texture** `.ktx2` (standard Khronos) : formats GPU, mips, couches (frames d'animation), faces de cubemap. Les VTF DXT sont copiés tels quels (sans perte ni réencodage), les autres encodés en BC7 (`intel_tex_2`).
- **Conversion des lumières** : les intensités des lumières (`_light`) sont mises à l'échelle pour qu'une surface blanche éclairée à 1 apparaisse blanche. Le pitch d'une entité lumière est positif vers le haut (contrairement aux angles de vue), qu'il vienne de `pitch` ou d'`angles` ; `angle` -1/-2 = haut/bas.
- **Luminaires** : les maps placent leurs lumières dans des modèles (globes, abat-jour, boîtiers). Pour chaque lumière, le convertisseur cherche un petit prop (192 unités au plus) dont la boîte la contient ; la lumière reçoit une **marge** (au moins 32 unités, sinon jusqu'au coin le plus éloigné du luminaire) que les rayons d'ombre ignorent, au rendu comme dans le bake.
- **Lumière indirecte : baker GPU** (`refract-render`, module `gi`, appelé par le convertisseur) :
  - une grille de sondes sur le monde jouable (jusqu'à 2 M sondes, au moins 64 unités entre deux) ;
  - lancer de rayons matériel (ray query de wgpu, expérimental, Vulkan) contre une structure d'accélération du monde, des brushes cachés et des props opaques, chaque triangle avec l'albédo moyen de son matériau ;
  - par sonde, 64 rayons (Fibonacci sphérique, tournés à chaque passe) ; à l'impact : soleil et lumières proches (triées par une grille de cellules de 512 unités) avec rayons d'ombre, plus la lumière des sondes de la passe précédente ; sans impact : le ciel. 3 passes = 3 rebonds ;
  - par sonde : un ambient cube, la distance libre moyenne dans les 6 directions, et « valide » si moins d'un quart des rayons touchent des faces arrière (sinon la sonde est dans un mur). Les surfaces fines (double face ou alpha test : feuillages, grillages) sont éclairées des deux côtés et leur dos ne compte pas comme une face arrière, sinon les sondes dans les arbres seraient « dans un mur » ;
  - les sondes dans les murs reçoivent ensuite la lumière de leurs voisines valides (3 passes de dilatation), tout en restant marquées non valides ;
  - petite map : 2 M sondes en 0,7 s ; grande map de ville : 2 M sondes sur 3,7 M triangles en 1,7 s (RTX 4070 Ti SUPER) ;
  - sans GPU capable de lancer des rayons, la même grille est remplie avec les ambient cubes du BSP (sans visibilité, cellules vides comblées par leurs voisines).
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
- **Maps** : la géométrie d'une map (ou d'un modèle) est envoyée en une fois depuis le mmap ; chaque bloc ou sous-maillage devient une instance. Textures **bindless** (un tableau de toutes les textures, indexé par matériau ; groupe de bind séparé, wgpu ne mélangeant pas tableaux et uniform buffers), échantillonnage anisotrope 16×. Matériaux opaques, alpha test, double face, transparents et additifs (quatre pipelines, dessinés dans cet ordre ; transparents pas encore triés). Lumière indirecte : les sondes cuites (voir §4). Champ de vision vertical 75°. Tonemapping **Khronos PBR Neutral**, qui garde les couleurs des textures telles qu'elles ont été peintes.
- **Lumières en clusters** : la vue est découpée en 16 × 9 tuiles × 24 tranches de profondeur (exponentielles jusqu'à 16384 unités) ; un compute shader liste dans chaque cluster les lumières dont la portée l'atteint (128 au plus), et chaque pixel ne boucle que sur celles de son cluster. Le CPU n'envoie que les lumières visibles (jusqu'à 4096). Les lumières qui demandent des ombres se partagent le tableau d'ombres locales ; **toutes les autres ont des ombres par rayons** quand le GPU lance des rayons en matériel : pour chaque lumière qui touche un pixel, un rayon d'ombre (ray query) contre les surfaces opaques de la scène, en ignorant la marge du luminaire de la lumière (voir §4). Le moteur construit lui-même ces structures d'accélération (une par maillage, une qui place les instances opaques projetant des ombres, reconstruite quand elles changent). Sans ce matériel, ou avec `-noraytracing`, le shader est compilé sans cette variante (`// variant:` dans `scene.slang`, géré par `build.rs`) et les lumières de map traversent les murs. Coût sur une grande map de ville : de 0,2 à 1 ms par image.
- **Passe de profondeur préalable** : les surfaces opaques écrivent d'abord leur profondeur (les surfaces en alpha test à travers leur texture), puis sont ombrées avec un test « égal » : chaque pixel n'est ombré qu'une fois. Les deux passes transforment les positions par les mêmes fonctions de shader, donc trouvent exactement les mêmes profondeurs.
- **Ombres des surfaces en alpha test** : un pipeline d'ombre à part lit leur texture et ne garde que les texels opaques (feuillages, grillages).

Mesures (scène de départ, 1280×720, `--bench`) : petite map 0,3 ms de CPU et ≈ 1,2 ms par image ; grande map de ville 1,6 ms de CPU et ≈ 4,3 ms par image avec ses 1027 lumières. Conversion complète de la grande map : 6 s la première fois, ≈ 2 s pour la map seule (dont 1,7 s de bake) ; chargement 1,5 s initialisation du GPU comprise.

- **Surfaces** : normal maps (repère tangent par sommet, la normale géométrique place les lectures d'ombre et pondère les sondes, la normale de la texture éclaire), seconde couche des displacements (mélange par l'alpha des sommets, affiné par la texture de modulation), textures de détail.
- **Brouillard** : brouillard de hauteur exponentiel, calculé en forme close dans le shader de la scène et celui du fond (quelques instructions par pixel, ni marche dans un volume ni passe supplémentaire). Sa densité décroît avec l'altitude (d'un facteur e tous les 4096 unités au-dessus du point d'apparition), ce qui voile l'horizon et laisse le zénith dégagé ; sa lumière est sa couleur (le ciel) plus 30 % de soleil diffusé vers l'avant (Henyey-Greenstein, g = 0,6), qui fait briller la brume autour du soleil. Le convertisseur traduit le brouillard linéaire de la map (`env_fog_controller` : début, fin, densité max, couleur affichée à 1,5× comme le ciel) : le brouillard est à moitié opaque là où l'ancien était à mi-chemin, puis s'épaissit doucement au lieu de buter sur un mur à la distance de fin. Le décor lointain (skybox 3D, instances marquées `set_distant`) et le ciel prennent la densité du brouillard de la skybox (`sky_camera`, distances multipliées par son échelle). Les surfaces additives ne font que s'estomper.

À venir pour la parité : défilement de la couche de nuages du ciel peint, masques des reflets (`$envmapmask`…) et textures dans les reflets, eau, sprites, proxies de matériaux, distance d'affichage des props, ordre de profondeur de la skybox 3D.

## 6. Feuille de route

1. **Structure** : `engine/` + `tools/`. ✅
2. **Nouveau rendu**, première étape : wgpu + Slang, PBR, lumières et ombres dynamiques, sur une scène de test. ✅
3. **Format natif** (`refract-asset`). ✅
4. **`refract-convert`** : BSP → format natif ; le client charge les maps converties. ✅
5. **Nouveau rendu** jusqu'à la parité avec l'ancien visualiseur par captures (liste en §5).
6. **Import VMF** + rechargement en direct, puis **baker d'éclairage GPU**.
7. **Couche jeu** : `world` (entités, tick), physique, LuaJIT, puis réseau et serveur dédié.
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

- Physique (`rapier3d` ou Jolt) et transport réseau
- Authentification (Steam ou comptes maison), hébergement des addons
- Licence du projet
