# Hardware-faithful LLE

We emulate the SCPH-1001 by executing its real BIOS and the hardware as documented, not by HLE of BIOS services. Timing is the crystal; every device moves on it (ADR 0011).

First proof is a *visible* Intro through to the Shell. That surface is frozen until the golden passes:

- **IN:** CPU interpreter, COP0, GTE; 2 MiB RAM, BIOS, scratchpad, memctrl, I/O; IRQ, DMA (GPU, OTC, CD-ROM, SPU), timers, vblank; GPU into VRAM and Display area; CD-ROM Init/GetStat/GetID/Nop reporting no disc; SPU that completes status/DMA so the BIOS does not hang, with no host audio; controllers and memory cards reporting nothing connected; 4 KiB i-cache such that `FlushCache` works.
- **OUT:** MDEC, disc sector reads, CD-XA, host audio, dynarec, wgpu-as-GPU, any machine that is not SCPH-1001.

The boot jingle (host audio) is the next proof, still before any disc. A dynarec is not in scope until a title is host-bound and the interpreter is the proven bottleneck.
