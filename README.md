# rsx

PlayStation 1 emulator. You supply the BIOS (`SCPH1001.BIN`); it is not in the repo. A disc is optional (CUE/BIN).

```
devenv allow
devenv shell -- cargo run -p rsx --release -- --bios SCPH1001.BIN
devenv shell -- cargo run -p rsx --release -- --bios SCPH1001.BIN --disc game.cue
```

`--bios` / `--disc` can also be set as `bios` / `disc` in `rsx.toml`:

```toml
bios = "SCPH1001.BIN"
disc = "game.cue"
```
