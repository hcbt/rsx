# wgpu presents the PlayStation image; egui is the Debugger

wgpu is the host GPU (Metal on macOS, other backends later). The PlayStation GPU software-rasterizes into VRAM; each vblank we copy the Display area into a wgpu texture. wgpu is not a stand-in for the guest GPU. egui is a Debugger-first host UI: framebuffer view, run/pause/step, logs, inspectors. A chrome-hidden player mode comes after the hardware works.
