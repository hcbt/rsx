# Hardware-faithful LLE, event-timed

We emulate the SCPH-1001 by executing its real BIOS and the hardware as documented, not by HLE of BIOS services. Timing is event-based (vblank, timer IRQ, DMA completion), not cycle-accurate from day one; we tighten timing when a test or a title demands it.

First proof is a *visible* Intro through to the Shell: R3000A interpreter including GTE, software GPU into VRAM, COP0, IRQ, DMA, timers, memory map, and a CD-ROM that reports no disc. SPU may be silent; the boot jingle is the next proof, still before any disc. A dynarec is not in scope until a title is host-bound and the interpreter is the proven bottleneck. Other regions, motherboard revisions, and peripherals stay out of scope until this machine is solid.
