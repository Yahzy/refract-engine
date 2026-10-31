# Contributing

Contributions are welcome: bug reports, ideas, and pull requests.

## Before you start

- For anything larger than a small fix, open an issue first to discuss the approach.
- Keep the structure clean: `engine/*` never depends on `tools/*`, dependencies go one way, one crate has one job (see `docs/DESIGN.md`).
- Run `cargo test --workspace` before opening a pull request. For rendering changes, include a before/after screenshot (`--screenshot out.png`).
- Code, comments, names and commit messages are in English.

## License of contributions

Refract is released under the [PolyForm Noncommercial License 1.0.0](LICENSE.md): anyone may use, study, modify and share it for noncommercial purposes, but not sell it or use it commercially.

By submitting a contribution (code, documentation, shaders, assets or anything else), you agree that:

1. you wrote it yourself, or otherwise have the right to submit it;
2. it is licensed to everyone under the same PolyForm Noncommercial License 1.0.0;
3. you also grant the project owner (Yahzy) a perpetual, worldwide, irrevocable, royalty-free license to use, modify, sublicense and relicense your contribution under any terms, including commercial ones.

Point 3 lets the project keep a single owner who can change the license or offer commercial licenses later, without having to track down every contributor. You keep the copyright on your work.
