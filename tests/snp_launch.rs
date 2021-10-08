// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, Instant};

use rand::{thread_rng, Rng};

use sev::{firmware::Firmware, launch::snp::*};

pub use kvm_bindings::kvm_segment as KvmSegment;
use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::{Kvm, VcpuExit};
use mmarinus::{perms, Kind, Map};

// one page of `hlt`
const CODE: &[u8; 4096] = &[
    0xf4; 4096 // hlt
];

const MEM_ADDR: u64 = 0x1000;

fn run_bench_update_data(size: usize, page_type: SnpPageType) -> Duration {
    let kvm_fd = Kvm::new().unwrap();
    let vm_fd = kvm_fd.create_vm().unwrap();

    let mut code_address_space = Map::map(CODE.len())
        .anywhere()
        .anonymously()
        .known::<perms::ReadWrite>(Kind::Private)
        .unwrap();

    code_address_space[..CODE.len()].copy_from_slice(&CODE[..]);

    let code_mem_region = kvm_userspace_memory_region {
        slot: 0,
        guest_phys_addr: MEM_ADDR,
        memory_size: code_address_space.size() as _,
        userspace_addr: code_address_space.addr() as _,
        flags: 0,
    };

    unsafe {
        vm_fd.set_user_memory_region(code_mem_region).unwrap();
    }

    let mut data_address_space = Map::map(size)
        .anywhere()
        .anonymously()
        .known::<perms::ReadWrite>(Kind::Private)
        .unwrap();
    if page_type != SnpPageType::Zero {
        thread_rng().fill(&mut data_address_space[..size]);
    }

    let data_mem_region = kvm_userspace_memory_region {
        slot: 1,
        guest_phys_addr: MEM_ADDR + CODE.len() as u64,
        memory_size: data_address_space.size() as _,
        userspace_addr: data_address_space.addr() as _,
        flags: 0,
    };

    unsafe {
        vm_fd.set_user_memory_region(data_mem_region).unwrap();
    }

    let sev = Firmware::open().unwrap();
    let launcher = Launcher::new(vm_fd, sev).unwrap();

    let start = SnpStart {
        policy: SnpPolicy {
            flags: SnpPolicyFlags::SMT,
            ..Default::default()
        },
        ..Default::default()
    };

    let imi_page = false;
    // If VMPL is not enabled, perms must be zero
    let dp = VmplPerms::empty();
    let perms = (dp, dp, dp);

    let update_code = SnpUpdate::new(
        code_mem_region.guest_phys_addr >> 12,
        code_address_space.as_ref(),
        imi_page,
        SnpPageType::Normal,
        perms,
    );

    let update_data = SnpUpdate::new(
        data_mem_region.guest_phys_addr >> 12,
        data_address_space.as_ref(),
        imi_page,
        page_type,
        perms,
    );

    let finish = SnpFinish::new(None, None, [0u8; 32]);

    let start_time = Instant::now();

    let mut launcher = launcher.start(start).unwrap();

    launcher.update_data(update_code).unwrap();
    launcher.update_data(update_data).unwrap();

    let vcpu_fd = launcher.as_mut().create_vcpu(0).unwrap();

    let mut regs = vcpu_fd.get_regs().unwrap();
    regs.rip = MEM_ADDR;
    regs.rflags = 2;
    vcpu_fd.set_regs(&regs).unwrap();

    let mut sregs = vcpu_fd.get_sregs().unwrap();
    sregs.cs.base = 0;
    sregs.cs.selector = 0;
    vcpu_fd.set_sregs(&sregs).unwrap();

    let (_vm_fd, _sev) = launcher.finish(finish).unwrap();

    let _ret = vcpu_fd.run();

    Instant::now().duration_since(start_time)
}

#[cfg_attr(not(has_sev), ignore)]
#[test]
fn bench_update_data() {
    for (size_str, size) in [
        ("1mb", 1 << 20),
        ("16mb", 1 << 24),
        ("32mb", 1 << 25),
        ("64mb", 1 << 26),
        ("128mb", 1 << 27),
    ] {
        for (page_str, page_type) in [
            ("normal", SnpPageType::Normal),
            ("zero", SnpPageType::Zero),
            ("unmeasured", SnpPageType::Unmeasured),
        ] {
            let duration = run_bench_update_data(size, page_type);
            println!(
                "{} {}: {:#?} ({} KB/ms)",
                page_str,
                size_str,
                duration,
                (size as f64 / duration.as_nanos() as f64) * 1_000 as f64 // KB / ms 
            );
        }
    }
}

#[cfg_attr(not(has_sev), ignore)]
#[test]
fn snp() {
    let kvm_fd = Kvm::new().unwrap();
    let vm_fd = kvm_fd.create_vm().unwrap();

    let mut address_space = Map::map(CODE.len())
        .anywhere()
        .anonymously()
        .known::<perms::ReadWrite>(Kind::Private)
        .unwrap();

    address_space[..CODE.len()].copy_from_slice(&CODE[..]);

    let mem_region = kvm_userspace_memory_region {
        slot: 0,
        guest_phys_addr: MEM_ADDR,
        memory_size: address_space.size() as _,
        userspace_addr: address_space.addr() as _,
        flags: 0,
    };

    unsafe {
        vm_fd.set_user_memory_region(mem_region).unwrap();
    }

    let sev = Firmware::open().unwrap();
    let launcher = Launcher::new(vm_fd, sev).unwrap();

    let start = SnpStart {
        policy: SnpPolicy {
            flags: SnpPolicyFlags::SMT,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut launcher = launcher.start(start).unwrap();

    // If VMPL is not enabled, perms must be zero
    let dp = VmplPerms::empty();

    let update = SnpUpdate::new(
        mem_region.guest_phys_addr >> 12,
        address_space.as_ref(),
        false,
        SnpPageType::Normal,
        (dp, dp, dp),
    );

    launcher.update_data(update).unwrap();

    let finish = SnpFinish::new(None, None, [0u8; 32]);

    let vcpu_fd = launcher.as_mut().create_vcpu(0).unwrap();

    let mut regs = vcpu_fd.get_regs().unwrap();
    regs.rip = MEM_ADDR;
    regs.rflags = 2;
    vcpu_fd.set_regs(&regs).unwrap();

    let mut sregs = vcpu_fd.get_sregs().unwrap();
    sregs.cs.base = 0;
    sregs.cs.selector = 0;
    vcpu_fd.set_sregs(&sregs).unwrap();

    let (_vm_fd, _sev) = launcher.finish(finish).unwrap();

    let ret = vcpu_fd.run();

    assert!(matches!(ret, Ok(VcpuExit::Hlt)));
}
