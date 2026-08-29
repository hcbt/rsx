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

**SPU**:
The PlayStation sound processor, with its own 512 KiB RAM. Silent in the first proof; the Intro jingle is the next proof.
_Avoid_: audio, mixer

**CD-ROM**:
The bus controller the BIOS talks to (status, GetID), not a disc image. First proof reports no disc; it does not read sectors.
_Avoid_: drive, ISO, disc as the name for the controller
