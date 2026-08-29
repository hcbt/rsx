# The Machine is a library; the Debugger is a binary

The emulated PlayStation is a host-window-free library: load BIOS, reset, step, inspect registers and VRAM. wgpu and egui never enter it. The Debugger is a separate binary that presents a Machine. From the first emulating commit, tests step the Machine headless.

First-proof tests run a fixed number of vblanks then hash the Display area. The first golden is captured from our run after the image is accepted by eye. We do not take goldens from another emulator.
