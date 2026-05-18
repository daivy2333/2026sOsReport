use std::arch::naked_asm;

#[derive(Debug, Default)]
#[repr(C)]
pub struct ThreadContext {
    pub rsp: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
}

#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn switch(old: *mut ThreadContext, new: *const ThreadContext) {
    naked_asm!(
        "mov [rdi + 0x00], rsp",
        "mov [rdi + 0x08], r15",
        "mov [rdi + 0x10], r14",
        "mov [rdi + 0x18], r13",
        "mov [rdi + 0x20], r12",
        "mov [rdi + 0x28], rbx",
        "mov [rdi + 0x30], rbp",
        "mov rsp, [rsi + 0x00]",
        "mov r15, [rsi + 0x08]",
        "mov r14, [rsi + 0x10]",
        "mov r13, [rsi + 0x18]",
        "mov r12, [rsi + 0x20]",
        "mov rbx, [rsi + 0x28]",
        "mov rbp, [rsi + 0x30]",
        "ret"
    );
}

pub unsafe fn init_stack(
    stack: &mut [u8],
    ctx: &mut ThreadContext,
    f: usize,
    guard: usize,
    skip: usize,
) {
    let len = stack.len();
    let s_ptr = stack.as_mut_ptr().add(len);
    let s_ptr = (s_ptr as usize & !15) as *mut u8;
    std::ptr::write(s_ptr.offset(-16) as *mut u64, guard as u64);
    std::ptr::write(s_ptr.offset(-24) as *mut u64, skip as u64);
    std::ptr::write(s_ptr.offset(-32) as *mut u64, f as u64);
    ctx.rsp = s_ptr.offset(-32) as u64;
}
