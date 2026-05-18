//! src: https://github.com/systemxlabs/green-threads-in-200-lines-of-rust

use std::arch::naked_asm;

#[derive(Debug, Default)]
#[repr(C)]
pub struct ThreadContext {
    pub ra: u64,
    pub sp: u64,
    pub s0: u64,
    pub s1: u64,
    pub s2: u64,
    pub s3: u64,
    pub s4: u64,
    pub s5: u64,
    pub s6: u64,
    pub s7: u64,
    pub s8: u64,
    pub s9: u64,
    pub s10: u64,
    pub s11: u64,
    pub entry: u64,
}

#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn switch(old: *mut ThreadContext, new: *const ThreadContext) {
    naked_asm!(
        "
        sd ra, 0*8(a0)
        sd sp, 1*8(a0)
        sd s0, 2*8(a0)
        sd s1, 3*8(a0)
        sd s2, 4*8(a0)
        sd s3, 5*8(a0)
        sd s4, 6*8(a0)
        sd s5, 7*8(a0)
        sd s6, 8*8(a0)
        sd s7, 9*8(a0)
        sd s8, 10*8(a0)
        sd s9, 11*8(a0)
        sd s10, 12*8(a0)
        sd s11, 13*8(a0)
        sd ra, 14*8(a0)

        ld ra, 0*8(a1)
        ld sp, 1*8(a1)
        ld s0, 2*8(a1)
        ld s1, 3*8(a1)
        ld s2, 4*8(a1)
        ld s3, 5*8(a1)
        ld s4, 6*8(a1)
        ld s5, 7*8(a1)
        ld s6, 8*8(a1)
        ld s7, 9*8(a1)
        ld s8, 10*8(a1)
        ld s9, 11*8(a1)
        ld s10, 12*8(a1)
        ld s11, 13*8(a1)
        ld t0, 14*8(a1)

        jr t0
        "
    );
}

pub unsafe fn init_stack(
    stack: &mut [u8],
    ctx: &mut ThreadContext,
    f: usize,
    guard: usize,
    _skip: usize,
) {
    let len = stack.len();
    let s_ptr = stack.as_mut_ptr().add(len);
    let s_ptr = (s_ptr as usize & !7) as *mut u8;
    ctx.ra = guard as u64;
    ctx.sp = s_ptr as u64;
    ctx.entry = f as u64;
}
