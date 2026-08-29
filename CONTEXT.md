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
