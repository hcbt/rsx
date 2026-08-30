# PlayStation

We emulate the SCPH-1001 NTSC-U PlayStation. This glossary is the guest machine and the host Debugger, not host libraries.

## Language

**PlayStation**:
The SCPH-1001 NTSC-U console this project emulates.
_Avoid_: PSX, PSOne, PS1 as the canonical name (PSOne is a later slim model; PSX is overloaded)

**BIOS**:
The SCPH-1001 firmware, a 512 KiB image supplied by the user and never part of the repository.
_Avoid_: ROM, firmware dump

**Intro**:
The BIOS boot sequence before the Shell: the Sony Computer Entertainment animation and, on hardware, the boot jingle.
_Avoid_: splash, BIOS movie

**Shell**:
The BIOS program after the Intro: the "insert PlayStation CD-ROM" screen and the memory-card and CD-player menus.
_Avoid_: BIOS menu, OSD, dashboard, shell as a name for the host UI

**Debugger**:
The host egui UI for inspecting and controlling the emulated PlayStation.
_Avoid_: shell, frontend, GUI

**Machine**:
The emulated PlayStation as a host-window-free object: load BIOS, reset, step, inspect. It does not know about wgpu or egui.
_Avoid_: emulator, core, engine as the canonical name

**CPU**:
The LSI CW33300, a MIPS R3000A-compatible processor with COP0 and GTE, no FPU, no TLB.
_Avoid_: MIPS as the product name

**GTE**:
COP2 on the CPU: transforms and lighting that feed GPU primitives. The Intro uses it.
_Avoid_: GPU (it is not the GPU)

**GPU**:
The PlayStation graphics processor. It draws into VRAM via its own command stream.
_Avoid_: wgpu, renderer, host GPU

**VRAM**:
The GPU's 1 MiB of memory, 1024×512 16-bit texels, holding framebuffer and textures. Not on the CPU bus.
_Avoid_: framebuffer as the name for the whole memory

**Display area**:
The rectangle of VRAM the GPU currently outputs as the visible picture.
_Avoid_: screen, framebuffer when you mean this rectangle

**Capture**:
A host PNG of the Display area, taken by the Debugger (`--capture-at`, or the Capture display control). This is how the picture is inspected without sitting in the window.
_Avoid_: screenshot, dump, framebuffer grab as the canonical name

**Clock**:
The SCPH-1001 master crystal at 33.8688 MHz. CPU steps, GPU vblank (263 lines × 2160 cycles), and SPU samples (768 cycles) all count this. Host realtime is `origin_cycles + elapsed_ns × CPU_HZ / 1e9`. The Debugger runs the Machine until that cycle and presents the current Display area and PCM. The audio device is a speaker, not the clock.
_Avoid_: audio buffer as master clock, vsync as the guest clock, one-vblank-then-wait as the realtime driver

**SPU**:
The PlayStation sound processor, with its own 512 KiB RAM. It mixes 24 ADPCM voices to 44100 Hz stereo PCM (one pair per 768 master cycles). The Debugger plays that and `--capture-at` writes `audio.wav`. CD-XA and reverb are not implemented.
_Avoid_: audio, mixer

**CD-ROM**:
The bus controller the BIOS talks to (status, GetID, Read).
_Avoid_: drive, ISO as the name for the controller

**Disc**:
A user-supplied CD-ROM image (CUE/BIN), never part of the repository. Optional: missing means the drive is empty.
_Avoid_: ROM, ISO as the name for the image
