# Instruction cache is real enough for FlushCache

The CPU has a 4 KiB instruction cache. We model it enough that BIOS `FlushCache` after kernel relocate has the documented effect. A miss stalls for the remaining words of the line at that address's waitstates (ADR 0011). `FlushCache` is not a nop.
