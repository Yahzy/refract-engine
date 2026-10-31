# Refract

A small, modern game engine written in Rust, built to host community games: gamemodes, community servers and Lua modding.
Inspired by Garry's Mod and the Source Engine workflow.

*Can AI Slop do better than S&Box?*

## Goals

- **Simple**: few concepts and little code; every system has to earn its place.
- **Fast**: a renderer built directly on wgpu (Vulkan) and shaders written in Slang.
- **Multiplayer first**: fixed-tick simulation kept apart from rendering, stable entity ids, an authoritative server.
- **Moddable**: gameplay in sandboxed LuaJIT (planned).
- **Open to existing content**: maps, models, materials and textures (BSP, VMF, MDL, VMT, VTF, then glTF and FBX) are converted into native formats. The engine reads only those formats, and content is read from games the player has installed; nothing is redistributed.

## Status

Early work in progress. Today the engine:

- loads converted maps with PBR shading and a dynamic sun;
- draws thousands of map lights through clustered shading, with cascaded and ray-traced shadows;
- lights indirect bounces and reflections from a GPU light baker.

Gameplay, physics, networking and scripting come next.

## Building

Requirements: Rust (2024 edition), a Vulkan GPU, and the [Vulkan SDK](https://vulkan.lunarg.com/) for `slangc` (or set `SLANGC` to its path).

```
cargo build --release -p refract-client -p refract-convert
cargo run --release -p refract-client                   # test scene
cargo run --release -p refract-client -- +map <name>    # converts the map on first load, then plays it
```

Installed games to read content from are listed in `game/mount.toml` (local to each machine; format in `docs/DESIGN.md`).

Controls: hold the right mouse button to look around, WASD to move, E/Q to go up/down, Shift to go faster, Esc to quit.

## Layout

```
engine/   the engine: core, asset formats, platform, renderer, client
tools/    content tools: format readers and the converter
docs/     design document (in French)
game/     development game folder
```

## License

MIT
