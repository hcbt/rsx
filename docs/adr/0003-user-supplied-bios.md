# User-supplied BIOS, never in git

The SCPH-1001 BIOS is Sony's copyrighted firmware. This public repository never contains it.

The Machine always receives an explicit filesystem path; it does not search the working directory. The Debugger takes that path from either `--bios <path>` or `rsx.toml` (`bios = "..."`). Either is enough. If both are set, the CLI wins. Missing file, or not exactly 512 KiB: do not emulate, and show the error in the Debugger.
