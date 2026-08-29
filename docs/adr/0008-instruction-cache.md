# Instruction cache is real enough for FlushCache

The CPU has a 4 KiB instruction cache. We model it enough that BIOS `FlushCache` after kernel relocate has the documented effect. We do not model cycle-accurate cache fill or stall timing. `FlushCache` is not a nop.
