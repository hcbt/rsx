# Cycle-accurate concurrent LLE

SCPH-1001. The 33.8688 MHz crystal is the only clock. CPU, bus, DMA, GPU (FIFO, draw, CRT beam), SPU, CD-ROM, and timers all move on that clock. This spec supersedes ADR 0001 (event-timed) and ADR 0008 (i-cache without stall cycles). Numbers come from PSX-SPX unless a line says SPX does not specify.

Host realtime (wall vs `CPU_HZ`) is already in place. This spec is the guest.

## Time

One guest cycle is one tick of the crystal (`CPU_HZ = 33_868_800`).

At every cycle every device has a defined state. DMA can own the bus during a load. The CRT can latch a line during that occupancy. The SPU mixes on cycle `n` where `n % 768 == 0`, whether or not a CPU instruction finished.

The host may skip a run of cycles only when nothing observable happens in that run: no bus grant, no IRQ edge, no SPU sample, no scanline latch, no GPU fragment, no CD IRQ, CPU only issuing cached ALU with no outstanding bus. Skipping idle time is not an event-delay. A counter that fires IRQ3 after a flat 256 cycles is.

`Machine::run_until_cycle(target)` still means “advance the crystal to `target`.” Internally that is a loop of ticks, with idle-skip where the skip is identical to ticking 1, 1, 1.

`Machine::step()` executes until one instruction has issued and its issue occupancy is accounted, then returns. Tests that today assume `cycles() - c0 == 2` after one ALU instruction change to `== 1` (SPX: ALU issue is 1 cycle).

## Host (Debugger)

Guest master, host presents. Already: `target = origin_cycles + elapsed_ns × CPU_HZ / 1e9`.

Remove:

- 4 ms `QUANTUM`. Sleep `wait_for_wall`. If that duration is zero, sleep one SPU sample of wall time: `768 × 1e9 / CPU_HZ` nanoseconds. Derived from the crystal, not a guessed floor.
- 1 s speaker cap and 120 s SPU capture cap. Queues grow. Capture already drains every vblank. Underrun is silence.
- Hold-resample. Open 44100 Hz stereo. If the device refuses, log and stay silent. Do not play at another rate.

Headless `--capture-at` still runs full-speed to a vblank count.

## CPU and bus

R3000A issue vs occupancy, from [CPU Specifications](https://psx-spx.consoledev.net/cpuspecifications/) and [Memory Control](https://psx-spx.consoledev.net/memorycontrol/).

Issue (cached, no bus):

| Op | Cycles |
| --- | --- |
| ALU, shift, lui, jump/branch issue | 1 |
| Store that fits in the write-queue | 1 |
| `mult`/`multu` start | 1 (result later) |
| `div`/`divu` start | 1 (result later) |

`mfhi`/`mflo` stall until the mul/div unit is done:

| Op | Cycles until HI/LO ready |
| --- | --- |
| `multu` | 6 / 9 / 13 from `rs` magnitude (SPX table) |
| `mult` | 6 / 9 / 13 from signed `rs` (SPX table) |
| `div`/`divu` | 36 |

Load issue is 1. Occupancy depends on the target (SPX, hardware-measured, includes the issue cycle):

| Target | Cycles |
| --- | --- |
| Scratchpad `1F800000h` | 1 |
| On-die I/O (IRQ, DMA, timers) | 5 |
| Main RAM | 7 (DRAM refresh may add; ignore refresh until a title shows it) |
| BIOS ROM | 27–33 from the delay/size registers at `1F801010h` plus COM_DELAY |

The bus occupancy overlaps following instructions that do not use the loaded register and do not start another bus access. A following independent nop does not add a full extra occupancy on top of the load. A following use of `rt` stalls until occupancy ends. A following load/store/I/O stalls until the bus is free.

Stores go to a 4-entry write-queue. A fifth store, or a read from RAM/I/O, stalls until a slot (or the bus) is free.

BIOS/EXP/SPU/CD delay registers use the SPX bitfields (write delay, read delay, COM0–3). Do not hardcode 27. Compute from the registers the BIOS actually wrote.

I-cache (4 KiB, 256 lines × 4 words) already fills on miss. A miss stalls for the remaining words of the line, each word paying the waitstates of that address (RAM 7, BIOS from delay regs). `IBLKSZ` (BCC bits 8–9) selects 2-word vs 4-word refill; default is 4. KSEG1 and i-cache off already bypass the cache. `FlushCache` stays real.

GTE command execution time: SPX GTE chapter; stall the CPU on a GTE op for that many cycles. IRQs on a GTE command still skip `EPC+4` as they do now.

## DMA

[DMA Channels](https://psx-spx.consoledev.net/dmachannels/).

DMA owns the bus for the transfer. Rates:

| Channel | Clocks per word |
| --- | --- |
| 0 MDEC in, 1 MDEC out, 2 GPU, 6 OTC | 1 |
| 4 SPU | 4 |
| 3 CD-ROM (BIOS default) | 24 |
| 3 CD-ROM (games often set) | 40 |
| 5 PIO | 20 |

Plus DRAM hyper-page: about 17 clocks per 16 words of RAM (row load). CPU keeps running only from i-cache, scratchpad, COP0, GTE, and the write-queue. A RAM or I/O read, or a 5th write-queue push, stalls the CPU until this DMA slice finishes.

Chopping (SyncMode 0, CHCR bit 8): DMA window `1 << N` words, then CPU window `1 << M` cycles, repeat.

IRQ3: DICR flag on **completion of the transfer** (or per-slice if DICR bits 0–6). `I_STAT` bit 3 on DICR.31 0→1. No 256-cycle pad. The BIOS can arm after CHCR because the transfer itself takes longer than the CHCR write.

Linked-list DMA2 walks nodes at 1 clk/word plus GPU DREQ (FIFO not full).

## CD-ROM

[CDROM Drive](https://psx-spx.consoledev.net/cdromdrive/). Sector period stays 1×/2×: `CPU_HZ / 75` and half that (already 451_584 / 2). INT1 rate: `CPU_HZ * 0x930 / 4 / 44100` single speed, half for double (SPX).

The drive MCU (HC05) is not in this repo and will not be LLE’d. First and second responses use SPX **Response Timings** (33 MHz units on a PAL PSone; same counts on NTSC CPU clock):

| Event | Typical (hex cycles) |
| --- | --- |
| Nop INT3, motor on | `C4E1` |
| Nop INT3, motor off | `5CF4` |
| Init INT3 | `13CCE` |
| GetID INT2 | `4A00` |
| Pause INT2, 1× | `21181C` (~5 sectors) |
| Pause INT2, 2× | `10BD93` |
| Pause INT2, already paused | `1DF2` |
| Stop INT2, 1× / 2× | `D38ACA` / `18A6076` |

Seek second-response time depends on distance; SPX marks it unknown. Until a measured seek table exists, a Seek/Read that must move the head completes INT2 no sooner than one 1× sector (`451_584` cycles) and no later than the Stop 1× figure. Do not use `1000` / `2000` / `5000`.

BUSYSTS stays set until the command is accepted. INT3/INT2/INT1 queue as SPX describes (not OR’d together).

## SPU

Mix stays one stereo pair per 768 CPU cycles.

Volume sweep (voice/main volume bit 15) uses the same envelope as ADSR (SPX: Sweep Volume Mode). It starts at the current volume and runs to `+7FFFh` or `0000h`. No “hold `0x7FFF`”. Fixed volume (bit 15 = 0) stays `(reg as i16) << 1`.

Reverb and CD-XA remain out of scope (ADR 0001 OUT, still). Dry mix is enough for the Intro jingle.

## GPU

SCPH-1001 is a v0 GPU (CXD8514Q, 160-pin) on early boards or v2 on later; we emulate retail 1 MB VRAM, 1024×512, 15-bit draw.

### FIFO

GP0 has a 16-word FIFO. GP1 bypasses it. GP0(E3h–E5h) do not take FIFO space (SPX). GPUSTAT bits 26/28 follow SPX (cleared while the command is busy; polygon/line clear bit 28 on the command word, before vertices).

A command starts when all of its parameters are in the FIFO. Drawing does not finish in the same cycle the last parameter arrived.

### Draw

SPX does not publish cycles per fragment. Constraint: the rasterizer writes VRAM over time; GPUSTAT stays busy until those writes finish; a primitive cannot complete in the issue cycle. Fill rate: **one 15-bit pixel per CPU cycle** as the floor (GPU cannot be faster than 1 pixel/cycle on a single VRAM port). Textured pixels pay the extra VRAM reads (CLUT / texel) as extra cycles, one read per cycle. When a public hardware measurement with a license we can use gives a tighter number, replace the floor; do not invent a BIOS-shaped delay.

### CRT scanout

The GPU draws into VRAM. The television reads a **display** rectangle. Those are not the same buffer at the same instant.

NTSC (SCPH-1001):

- 263 scanlines per field, non-interlaced (SPX).
- Video clock 53.693175 MHz (SPX NTSC).
- 3413 video cycles per scanline (SPX).
- CPU cycles per scanline = `3413 * CPU_HZ / 53_693_175` (integer). HBLANK pulse once per line, including during VBLANK (Timer 1).

VBLANK for IRQ0 / GPUSTAT is outside the GP1(07h) vertical display range. GP1(00h) reset sets Y1=`010h`, Y2=`010h+240` (240 visible lines). HBLANK still fires every line, including during VBLANK (Timer 1).

**Latch:** at the start of each visible scanline, copy that line of the current Display area from VRAM into an output bitmap (the CRT). Lines not yet reached this field keep the previous field. During VBLANK the output is black if the display is off (GP1(03h)); if on, still no new visible lines. `Machine::display_area()` returns this output bitmap, not a live slice of VRAM.

Drawing into the not-yet-scanned region of the display rectangle can appear this field. Drawing into an already-latched line appears next field. That is the beam.

GP1(05h/06h/07h/08h) define origin, range, and mode. Interlace (GP1(08h) bit 5) latches even/odd lines per field; GPUSTAT bit 31 follows SPX.

## Tests (Machine seam)

Public `Machine` / `DisplayArea` / `CPU_HZ` / `cycles_in_nanos` / `run_until_cycle`. No poking of private DMA delay fields.

Independent expected values are SPX literals, not “whatever the code does”:

- One cached `addu` advances the crystal by 1.
- One `lw` from scratchpad advances by 1; from RAM by 7.
- `div` then immediate `mflo` stalls until 36.
- 768 cycles emit one SPU stereo frame (already true).
- DMA6 of `N` words takes about `N` clocks plus hyper-page, then IRQ3, with no extra 256.
- Overflow of the host PCM queue is not a test of the Machine; Debugger tests: 44100-only open path; `wait_for_wall` with no 4 ms floor except the 768-cycle sample when wait is zero.
- Scanout: after `run_until_cycle` of one line, `display_area()` contains the latched line and does not contain a GP0 fill that targeted a later line of the same field.

BIOS goldens (`run_until_vblank_count(n)` then hash) stay. Instruction counts per vblank will change; hashes may move. If a golden fails, capture again after the picture is accepted by eye. Do not restore `tick(2)` to keep a hash.

## Docs

- New ADR 0011: cycle-accurate concurrent LLE; crystal is the time unit; idle-skip only when identical to ticking one.
- ADR 0001: mark superseded by 0011 for timing. LLE/BIOS/IN-OUT list otherwise stands (MDEC, XA, dynarec still out).
- ADR 0008: i-cache miss pays waitstates.
- `CONTEXT.md` Clock: concurrent devices on the crystal; CRT latches the Display area per scanline.

## Out of scope

MDEC, CD-XA, reverb, dynarec, wgpu-as-GPU, HC05 firmware LLE, DRAM refresh stalls, v1 arcade GPU, 2 MB VRAM.

## Order of landing (one spec, sequential commits)

Each commit is one seam, tests first.

1. Host: drop QUANTUM / caps / hold-resample.
2. CPU issue 1 + mul/div stall + load occupancy + write-queue + computed BIOS waitstates.
3. I-cache miss stalls.
4. DMA rates, bus steal, IRQ on completion.
5. CD response times from the SPX table; keep 1×/2× sectors.
6. SPU volume sweep.
7. GPU FIFO + busy + draw-over-time.
8. CRT line latch from GP1 display range; `display_area()` is the beam.

Goldens after 2, 4, 5, 7, and 8.
