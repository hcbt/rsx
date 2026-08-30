# The Machine is a library; the Debugger is a binary

The emulated PlayStation is a host-window-free library: load BIOS from an explicit path, reset, inspect registers and VRAM. wgpu and egui never enter it. The Debugger is a separate binary that presents a Machine. From the first emulating commit, tests step the Machine headless.

The primitive is `step()`: one CPU instruction; timers, DMA, and GPU time advance with it. Convenience `run_until_vblank_count(n)` drives goldens, headless `--capture-at`, `--headless`, and the Debugger's "step frame". `run_until_cycle(n)` is Debugger realtime: wall time converted through `CPU_HZ`, never the host DAC. `--headless` is unpaced (same as `--capture-at`) and prints pace as host throughput. First-proof tests run a fixed number of vblanks then hash the Display area. The first golden is captured from our run after the image is accepted by eye. We do not take goldens from another emulator.
