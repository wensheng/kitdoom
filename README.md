# kitdoom

<img src="imgs/doom.png" width="300">

**Play Doom, with real graphics and sound, directly inside your terminal.**



## Install

    cargo install kitdoom

---

## Why

- Stop leaving the terminal just to scratch the "one quick level" itch.
- Skip emulator windows, launcher setup, and graphics backends you do not need.
- Keep Doom playable over a terminal workflow with the original framebuffer streamed as Kitty graphics.

---

<!-- ## Show, Don't Tell

![kitdoom demo placeholder](./assets/demo.gif)
-->

## Key Capabilities

- **Instant terminal carnage**: launch Doom from Cargo and drop straight into the shareware WAD.
- **Original pixels, modern pipe**: stream Doom's 640x400 framebuffer through compressed Kitty graphics chunks.
- **Sound included**: bundled effects and music play through the miniaudio bridge with no SDL setup.

---

## Usage


```bash
kitdoom
kitdoom -nosound
kitdoom -iwad /path/to/doom.wad
```

---

## How It Works

```text
doomgeneric C engine -> Rust FFI callbacks -> RGB framebuffer
        -> zlib + base64 Kitty chunks -> terminal
        -> crossterm input + miniaudio sound -> Doom loop
```

`kitdoom` keeps the engine proven and the terminal layer sharp: Rust owns the terminal lifecycle, input, scaling, timing, and Kitty rendering, while the vendored Doom core does what it already does best.
