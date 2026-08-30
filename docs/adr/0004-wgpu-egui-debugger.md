# wgpu presents the PlayStation image; egui is the Debugger

wgpu is the host GPU (Metal on macOS, other backends later). The PlayStation GPU software-rasterizes into VRAM; each vblank we copy the Display area into a wgpu texture. wgpu is not a stand-in for the guest GPU.

For first proof the Debugger shows: Display area, run/pause, step one instruction, step one frame (one vblank), a log of guest I/O and IRQs, CPU PC and GPRs, GPUSTAT, and host pace (guest cycles and vblanks per wall second versus the 33.87 MHz crystal and NTSC 59.62 Hz). VRAM viewer, GTE/DMA panels, disassembly, and breakpoints come after. A player mode that hides the Debugger and shows only the Display area comes after the hardware works.
