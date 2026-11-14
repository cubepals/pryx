// SPDX-FileCopyrightText: 2026 The Cubepals Authors
// SPDX-License-Identifier: FSL-1.1-ALv2

//! A disposable one-page guest executes HLT. The parent bounds the child lifetime, including
//! KVM_RUN; no host configuration, network, disk image or persistent guest is created.

#[cfg(target_arch = "x86_64")]
pub(super) fn halt() -> anyhow::Result<()> {
    use kvm_bindings::kvm_userspace_memory_region;
    use kvm_ioctls::{Kvm, VcpuExit};
    #[repr(C, align(4096))]
    struct Page([u8; 4096]);
    let mut memory = Box::new(Page([0; 4096]));
    // Intel's HLT instruction; a real-mode guest needs no interrupt controller to exit on it.
    memory.0[0] = 0xf4;
    let kvm = Kvm::new()?;
    let vm = kvm.create_vm()?;
    let region = kvm_userspace_memory_region {
        slot: 0,
        flags: 0,
        guest_phys_addr: 0x1000,
        memory_size: memory.0.len() as u64,
        userspace_addr: memory.0.as_mut_ptr() as u64,
    };
    #[expect(unsafe_code, reason = "KVM borrows this stable aligned guest page until the VM drops")]
    // SAFETY: the page is aligned, owned, writable and outlives vm and its vCPU; there is one slot.
    unsafe {
        vm.set_user_memory_region(region)?;
    }
    let mut cpu = vm.create_vcpu(0)?;
    let mut special = cpu.get_sregs()?;
    special.cs.base = 0;
    special.cs.selector = 0;
    cpu.set_sregs(&special)?;
    let mut total = std::time::Duration::ZERO;
    let samples = 10;
    for _ in 0..samples {
        let mut registers = cpu.get_regs()?;
        registers.rip = region.guest_phys_addr;
        registers.rflags = 2;
        cpu.set_regs(&registers)?;
        let start = std::time::Instant::now();
        anyhow::ensure!(matches!(cpu.run()?, VcpuExit::Hlt), "KVM did not exit on HLT");
        total += start.elapsed();
    }
    println!("Ran {samples} HLT exits; mean {} ns", total.as_nanos() / samples);
    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
pub(super) fn halt() -> anyhow::Result<()> {
    anyhow::bail!("The measured KVM guest probe is currently supported only on x86_64")
}
