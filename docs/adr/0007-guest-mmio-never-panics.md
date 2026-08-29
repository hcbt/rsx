# Guest MMIO never panics the host

Unimplemented or undocumented I/O follows PSX-SPX (open bus or RAZ/WI). We debug-log the access. The host process does not panic because the guest touched an address.
