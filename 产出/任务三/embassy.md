根据2025-2026年的最新资料，我对你之前的文档进行了更新与补充，力求保持原有的结构与风格，并加入了信息来源。以下是更新后的完整内容（Markdown格式）。

---

# Embassy：嵌入式异步运行时

Embassy 是一个为嵌入式设备设计的 Rust 异步运行时，支持 STM32、nRF、RP2040/RP2350、MSPM0 等众多微控制器系列。它在资源受限的环境中提供一流的 `async/await` 支持，结合中断与定时器实现高效的协作式调度，并通过编译时静态分配任务，实现零成本抽象。

## 设计背景

### 异步与阻塞

在执行 I/O 操作时，程序通常会在调用点阻塞，直到操作完成。在 Linux 等通用操作系统中，内核管理这类阻塞调用——内核可将 CPU 分配给其他线程，或使 CPU 进入睡眠模式。这类系统无法假设线程会主动协作，因此通过抢占式调度保证公平性。

若任务能主动协作，创建任务的资源消耗便可降至几乎可忽略。这种轻量级任务在 Rust 中通过 `async`/`await` 实现。

### Rust 异步模型

非阻塞操作由 `async`/`await` 实现。`async` 函数返回一个 `Future` 对象，当该 Future 因 I/O 操作而阻塞时，调度器（执行器）会选择其他 Future 执行。执行器无需轮询检测 Future 何时就绪，相比传统 RTOS 方案，异步拥有更好的切换性能和更低的消耗。不过，程序体积通常会比非异步程序更大——这在资源受限的嵌入式环境中需要权衡。在 Embassy 所支持的目标设备（如 STM32、nRF）上，内存通常足以容纳规模适度的程序。

## 项目组成

Embassy 项目由一组可自由组合的 crate 构成。截至 2026 年初，核心 crate 普遍发布了新的主版本，生态成熟度显著提升 \[^{1}\]。

### 执行器

`embassy-executor`（当前版本 v0.10.0）是一个专为嵌入式设计的异步执行器，在编译时分配固定数量的任务槽位。核心特性：

- **无需堆分配**：任务静态分配
- **动态大小**：无需配置即可支持 1 到 1000 个任务
- **集成定时队列**：通过 `Timer::after_secs(1).await` 实现休眠
- **无空转轮询**：无任务时配合中断或 WFE/WFI 使 CPU 进入休眠
- **高效轮询**：仅轮询被唤醒的任务，而非全部任务
- **公平调度**：即使某任务被频繁唤醒，也不会独占 CPU
- **多优先级**：支持创建多个执行器实例，以不同优先级运行任务
- **架构解耦**：定时器队列已独立为 `embassy-executor-timer-queue` crate，便于独立维护
- **新增平台**：新增对 NXP MCXA 系列 (`embassy-mcxa`) 的低功耗支持

### 硬件抽象层（HAL）

HAL 提供安全易用的接口来操作硬件，无需直接操作原始寄存器。当前维护的官方系列：

| crate | 目标硬件 |
|-------|---------|
| `embassy-stm32` | STM32 全系列（新增支持 STM32H5、STM32WB）|
| `embassy-nrf` | Nordic nRF52、nRF53、nRF91、nRF54 |
| `embassy-rp` | 树莓派 RP2040、RP2350 |
| `embassy-mspm0` | 德州仪器 MSPM0 系列 |
| `embassy-mcxa` | NXP MCXA 系列 |

此外，Espressif ESP32 系列由 `esp-rs` 社区维护，Wi-Fi 与蓝牙功能持续强化；WCH CH32V 等 RISC-V 芯片的支持也由社区提供。HAL 不依赖执行器，可在无异步的环境中使用，实现了 `embedded-hal` v0.2/v1.0 和 `embedded-io` 系列 trait。

### 网络栈

`embassy-net`（当前版本 v0.9.1）提供以太网、IP、TCP、UDP、ICMP、DHCPv4、DNS 与 IPv6 支持。异步在管理超时和同时处理多连接方面提供了极大便利。硬件驱动新增了 `embassy-net-wiznet` 与 `embassy-net-esp-hosted` 等。

### 蓝牙、LoRa、USB

- `nrf-softdevice`：为 nRF52 微控制器提供 BLE 4.x 和 5.x 支持
- `embassy-stm32-wpan`：为 STM32WB 系列提供低功耗蓝牙 5.x 支持
- `embassy-lora`：支持 STM32WL 无线微控制器和 Semtech SX127x 收发器上的 LoRa 网络
- `embassy-usb`：实现设备侧 USB 栈，支持 CDC ACM（USB 串行）、USB HID、CDC NCM（以太网 over USB）及自定义实现

### 启动引导

`embassy-boot` 是一个轻量级引导加载程序，支持断电保护的固件升级、试用引导和回滚。

## 环境准备

### 工具链

```bash
rustup update
cargo install cargo-embed   # 推荐使用 probe-rs 工具链
```

Embassy 生态的标准工具已更新为 `probe-rs` 及 `cargo-embed`，它们支持广泛的调试探针，并集成了 RTT 日志与 `defmt`。你也可以使用 `cargo-flash` 或 OpenOCD。

### Rust Nightly

Embassy 目前仍需要 nightly 工具链，因其使用了 `type_alias_impl_trait` 等不稳定特性：

```rust
#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
```

### 错误处理

开发阶段建议使用 `defmt_rtt` 和 `panic_probe` 将诊断信息输出到终端：

```rust
use {defmt_rtt as _, panic_probe as _};
```

无需专用开发板，可通过 `std` 特性在 PC 上运行 Embassy 应用。

## 代码示例：按钮控制 LED

以下通过四个递进层级的代码示例，展示从底层到异步的完整演进。

### PAC 版本（原始寄存器操作）

PAC（Peripheral Access Crate）是访问外设寄存器的最底层 API，提供类型安全的寄存器访问，但不阻止不安全代码。

```rust
#![no_std]
#![no_main]
use pac::gpio::vals;
use {defmt_rtt as _, panic_probe as _, stm32_metapac as pac};

#[cortex_m_rt::entry]
fn main() -> ! {
    let rcc = pac::RCC;
    unsafe { rcc.ahb2enr().modify(|w| { w.set_gpioben(true); w.set_gpiocen(true); });
             rcc.ahb2rstr().modify(|w| { w.set_gpiobrst(true); w.set_gpiocrst(true);
                                         w.set_gpiobrst(false); w.set_gpiocrst(false); }); }

    let gpioc = pac::GPIOC;
    const BUTTON_PIN: usize = 13;
    unsafe { gpioc.pupdr().modify(|w| w.set_pupdr(BUTTON_PIN, vals::Pupdr::PULLUP));
             gpioc.otyper().modify(|w| w.set_ot(BUTTON_PIN, vals::Ot::PUSHPULL));
             gpioc.moder().modify(|w| w.set_moder(BUTTON_PIN, vals::Moder::INPUT)); }

    let gpiob = pac::GPIOB;
    const LED_PIN: usize = 14;
    unsafe { gpiob.pupdr().modify(|w| w.set_pupdr(LED_PIN, vals::Pupdr::FLOATING));
             gpiob.otyper().modify(|w| w.set_ot(LED_PIN, vals::Ot::PUSHPULL));
             gpiob.moder().modify(|w| w.set_moder(LED_PIN, vals::Moder::OUTPUT)); }

    loop { unsafe {
        if gpioc.idr().read().idr(BUTTON_PIN) == vals::Idr::LOW {
            gpiob.bsrr().write(|w| w.set_bs(LED_PIN, true));
        } else { gpiob.bsrr().write(|w| w.set_br(LED_PIN, true)); }
    }}
}
```

需要手动启用外设时钟、配置每个寄存器。轮询按钮时忙等待，无法进入睡眠模式。

### HAL 版本

HAL 提供更高层次的 API 抽象，自动处理外设时钟和寄存器配置。

```rust
#![no_std]
#![no_main]
use cortex_m_rt::entry;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use {defmt_rtt as _, panic_probe as _};

#[entry]
fn main() -> ! {
    let p = embassy_stm32::init(Default::default());
    let mut led = Output::new(p.PB14, Level::High, Speed::VeryHigh);
    let button = Input::new(p.PC13, Pull::Up);
    loop {
        if button.is_low() { led.set_high(); } else { led.set_low(); }
    }
}
```

代码显著简化，但忙等待的问题仍然存在。

### 中断驱动版本

为节省能源，需通过中断通知事件，使 CPU 在空闲时进入睡眠。

```rust
#![no_std]
#![no_main]
use core::cell::RefCell;
use cortex_m::interrupt::Mutex;
use cortex_m_rt::entry;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::peripherals::{PB14, PC13};
use embassy_stm32::{interrupt, pac};
use {defmt_rtt as _, panic_probe as _};

static BUTTON: Mutex<RefCell<Option<Input<'static, PC13>>>> = Mutex::new(RefCell::new(None));
static LED: Mutex<RefCell<Option<Output<'static, PB14>>>> = Mutex::new(RefCell::new(None));

#[entry]
fn main() -> ! {
    let p = embassy_stm32::init(Default::default());
    let led = Output::new(p.PB14, Level::Low, Speed::Low);
    let mut button = Input::new(p.PC13, Pull::Up);
    cortex_m::interrupt::free(|cs| {
        enable_interrupt(&mut button);
        LED.borrow(cs).borrow_mut().replace(led);
        BUTTON.borrow(cs).borrow_mut().replace(button);
        unsafe { NVIC::unmask(pac::Interrupt::EXTI15_10) };
    });
    loop { cortex_m::asm::wfe(); }
}

#[interrupt]
fn EXTI15_10() {
    cortex_m::interrupt::free(|cs| {
        let mut button = BUTTON.borrow(cs).borrow_mut();
        let button = button.as_mut().unwrap();
        let mut led = LED.borrow(cs).borrow_mut();
        let led = led.as_mut().unwrap();
        if check_interrupt(button) {
            if button.is_low() { led.set_high(); } else { led.set_low(); }
        }
        clear_interrupt(button);
    });
}
```

程序复杂度显著上升——按钮和 LED 状态必须在全局范围保存，用 Mutex 保护。

### 异步版本（Embassy）

Embassy 执行器管理一组编译时定义的任务。当某任务阻塞时，执行器自动运行其他任务或让 CPU 进入睡眠。

```rust
#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
use embassy_executor::Spawner;
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());
    let mut led = Output::new(p.PB14, Level::Low, Speed::VeryHigh);
    let mut button = ExtiInput::new(Input::new(p.PC13, Pull::Up), p.EXTI13);
    loop {
        button.wait_for_any_edge().await;
        if button.is_low() { led.set_high(); } else { led.set_low(); }
    }
}
```

异步版本与 HAL 版本同样简洁，但多了两个关键行为：

- `wait_for_any_edge().await` 使当前任务挂起，若无其他任务可执行则进入睡眠
- Embassy HAL 自动配置了按钮中断，中断发生时唤醒等待的任务

四种方案的代码量对比：

| 方案 | 代码行数 | 忙等待 | 睡眠 | 复杂度 |
|------|---------|-------|------|-------|
| PAC | 60+ | 是 | 否 | 高 |
| HAL | 25 | 是 | 否 | 低 |
| 中断驱动 | 70+ | 否 | 是 | 最高 |
| Embassy Async | 25 | 否 | 是 | 低 |

## 执行器机制

### 执行流程

```
任务被轮询 → 尝试执行 → 遇到阻塞 → 返回 Pending
    ↑                              │
    │        其他任务执行完毕        │
    └──────────────────────────────┘
        被唤醒的任务重新入队
```

1. 任务被创建时，执行器对其执行 poll
2. 任务尝试运行，取得进展，直到被阻塞（等待 I/O、定时器等异步函数）
3. 当此情况发生时，任务返回 `Poll::Pending`
4. 执行器将任务重新加入运行队列末尾，继续执行下一任务
5. 任务完成或取消时不再入队

### 中断处理

中断是外设发出操作完成信号的通用机制，与异步执行模型高度适配：

1. 任务被轮询，尝试取得进展
2. 任务指示外设执行操作，并等待
3. 外设完成操作，发出中断
4. HAL 将中断信号路由到外设，更新外设状态
5. 执行器收到通知，任务可继续被轮询

`InterruptExecutor` 是一种由中断驱动的特殊执行器。通过创建多个实例，可为不同优先级的任务建立独立的调度域。

### 定时器

Embassy 维护一个内部定时器队列，启用 `time` feature 后可用：

- `Timer::after(Duration).await` 实现异步延迟
- 定时器频率可通过 `time-tick-<frequency>` 编译时配置（1000 Hz / 32768 Hz / 1 MHz）
- 嵌入式平台的定时器驱动可能仅支持固定数量的警报，使用时需注意不要超出限制
- 若应用不需要计时器，不启用 `time` 功能可节省 CPU 周期并降低功耗

## HAL 架构

### nRF 系列

`embassy-nrf` 底层已切换为基于 `chiptool` 的 `nrf-pac` PAC。支持 PWM、SPIM、QSPI、NVMC、GPIOTE、RNG、TIMER、WDT、TEMP、PPI、UARTE、TWIM、SAADC 等外设。定时器驱动默认工作频率为 32768 Hz。

### STM32 系列

STM32 微控制器家族庞大，外设版本在不同芯片间共享。Embassy 未针对每个芯片重复实现外设接口，而是通过自动派生特性标记匹配对应版本：

- `stm32-metapac` 编译时根据 Cargo feature 选择芯片定义
- 芯片和寄存器定义位于 `stm32-data` 模块
- 定时器驱动默认工作频率为 32768 Hz

## Bootloader

`embassy-boot` 设计用于断电安全的固件升级。

### 分区布局

| 分区 | 用途 |
|------|------|
| BOOTLOADER | 引导程序自身（约 8-24 KB） |
| ACTIVE | 主程序运行区 |
| DFU | 固件暂存区（大小 = ACTIVE + 一页） |
| BOOTLOADER STATE | 存储分区交换状态和进度 |

### 验证机制

支持 ed25519 数字签名验证。若验证启用，需使用 `FirmwareUpdater::verify_and_mark_updated` 代替 `mark_updated`，传入公钥、签名和固件实际长度。推荐 `ed25519-salty` 特性（占用闪存较小）。

密钥生成（Unix）：

```bash
signify -G -n -p $SECRETS_DIR/key.pub -s $SECRETS_DIR/key.sec
tail -n1 $SECRETS_DIR/key.pub | base64 -d | dd ibs=10 skip=1 > key.pub
chmod 700 $SECRETS_DIR/key.sec
```

---

## 参考资料

1. [Embassy 项目 GitHub 仓库](https://github.com/embassy-rs/embassy)  
3. [embassy-net v0.9.1 发布说明](https://github.com/embassy-rs/embassy/releases/tag/net-v0.9.1)  
4. [Embassy 官方文档与硬件支持列表](https://embassy.dev/)  
5. [probe-rs 调试工具](https://probe.rs/)  
6. [defmt 日志框架](https://defmt.ferrous-systems.com/)  
7. [esp-rs 社区对 ESP32 的 Embassy 支持](https://github.com/esp-rs/esp-hal)  
8. [embedded-hal 与 embedded-io 标准](https://github.com/rust-embedded/embedded-hal)  
9. [smoltcp 0.12.0 网络栈](https://github.com/smoltcp-rs/smoltcp)  

以上资料提供了最新版本、新增硬件支持及工具链变动的主要来源，搜集到这里当作参考。
或许可能会用到？