# Hardware-faithful LLE, event-timed

We emulate the SCPH-1001 by executing its real BIOS and the hardware as documented, not by HLE of BIOS services. Timing is event-based (vblank, timer IRQ, DMA completion), not cycle-accurate from day one; we tighten timing when a test or a title demands it. First proof the machine is alive is the BIOS Intro through to the Shell. Other regions, motherboard revisions, and peripherals stay out of scope until this machine is solid.
