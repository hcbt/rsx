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
The host egui UI for inspecting and controlling the emulated PlayStation. `--headless` is the same binary without the window: it runs the Machine unpaced and prints one measurement line per period on stdout (guest vblank/PC/GPU/DMA/CD, host pace).
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
The SCPH-1001 master crystal at 33.8688 MHz. CPU, bus, DMA, GPU (including the CRT), SPU, CD-ROM, and timers all move on this clock. Host realtime is `origin_cycles + elapsed_ns × CPU_HZ / 1e9`. The Debugger runs the Machine until that cycle and presents the current Display area and PCM. The audio device is a speaker, not the clock. **Pace** is guest cycles and vblanks per wall second versus that crystal (33.87 MHz) and NTSC vblank (`CPU_HZ / (2160 × 263)` ≈ 59.62 Hz). The Debugger shows it live and logs when the host falls behind 95% of the crystal. `--headless` prints that pace on stdout with the rest of the sample; it does not wait on wall time, so >100% is host throughput.
_Avoid_: audio buffer as master clock, vsync as the guest clock, event delay as the guest clock

**SPU**:
The PlayStation sound processor, with its own 512 KiB RAM. It mixes 24 ADPCM voices to 44100 Hz stereo PCM (one pair per 768 master cycles). The Debugger plays that and `--capture-at` writes `audio.wav`. CD-XA (Setmode bit 6, MODE2 audio+realtime) mixes into SPU analog; SPUCNT bit 7 runs the reverb work-area mix.
_Avoid_: audio, mixer

**CD-ROM**:
The bus controller the BIOS talks to (status, GetID, Read).
_Avoid_: drive, ISO as the name for the controller

**Disc**:
A user-supplied CD-ROM image (CUE/BIN), never part of the repository. Optional: missing means the drive is empty.
_Avoid_: ROM, ISO as the name for the image

**JOY**:
SIO0 at `1F801040h`. Slot 1 may be a standard digital pad (ID `5A41h`). Empty ports and slot 2 clock `0xFF` with no `/ACK`. A TX byte shifts at JOY_BAUD (8 × reload × factor); RX and TX Ready Flag 2 wait for last SCK. IRQ7 (`I_STAT.7`) follows `/ACK` after last SCK plus the kernel's 100-clk ignore window, not in the TX cycle, and not after the last digital Read byte.
_Avoid_: SIO1, DualShock analog mode as the first pad

**Pad**:
A standard digital controller on JOY slot 1. The Debugger maps a host DualShock 4 (Share→Select, Options→Start, face/shoulders/D-pad, left stick as D-pad) onto its active-low switches and injects that on present. On macOS the DualShock 4 is a Game Controller device, not IOKit HID. No host pad → slot 1 stays empty. The Machine does not open HID.
_Avoid_: rumble, analog ID `5A73h`, slot 2, keyboard-as-pad
