# wgpu presents the PlayStation image; egui is the Debugger

wgpu is the host GPU (Metal on macOS, other backends later). It displays the emulated framebuffer; it is not a stand-in for the PlayStation GPU. egui is a Debugger-first host UI: framebuffer view, run/pause/step, logs, inspectors. A chrome-hidden player mode comes after the hardware works.
