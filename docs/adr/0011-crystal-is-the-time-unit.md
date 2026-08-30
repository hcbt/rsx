# The crystal is the time unit

The SCPH-1001 master crystal at 33.8688 MHz is the only clock. CPU, bus, DMA, GPU (FIFO, draw, CRT), SPU, CD-ROM, and timers all move on it. At every cycle every device has a defined state. The host may skip a run of cycles only when nothing observable happens in that run. Skipping idle time is not an event delay.

Cycle counts come from PSX-SPX. We do not invent delays so the BIOS can arm an IRQ (a flat DMA pad, round CD command times, a fixed two cycles per instruction). Where SPX has no number (GPU fragment time, CD seek distance), the device stays busy until the work finishes and is not instant.

This replaces the event-timed rule in ADR 0001. An i-cache miss pays the waitstates of the filled words (ADR 0008). The Debugger still presents at wall time versus `CPU_HZ`; it does not pace the Machine with a host quantum, a speaker cap, or resampling.

The HC05 CD MCU is not LLE'd; CD responses use SPX Response Timings. MDEC, CD-XA, reverb, dynarec, and DRAM refresh stay out as in ADR 0001.
