#![no_std]
#![no_main]

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::Waker;
use cortex_m::peripheral::{syst::SystClkSource, SYST};
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::pac::{self};
use embassy_stm32::peripherals::PB7;
use embassy_time::{Timer};
use panic_halt as _;

use critical_section::{CriticalSection, Mutex};
use embassy_time_driver::Driver;
use embassy_time_queue_utils::Queue;

/*
use stm32f4xx_hal::{
    prelude::*,
};

 */

struct MyDriver {
    ticks: AtomicU32,
    queue: Mutex<RefCell<Queue>>,
    next: AtomicU32,
}

impl MyDriver {
    const fn new() -> Self {
        Self {
            ticks: AtomicU32::new(0),
            queue: Mutex::new(RefCell::new(Queue::new())),
            next: AtomicU32::new(0),
        }
    }

    /// Called from SysTick interrupt to increment ticks and process wakeups
    fn on_tick(&self) {
        let ticks = self.ticks.fetch_add(1, Ordering::Relaxed) + 1;
        let next = self.next.load(Ordering::Relaxed);
        if ticks >= next {
            critical_section::with(|cs| {
                let now = self.now();
                let mut queue = self.queue.borrow(cs).borrow_mut();
                self.next.store(queue.next_expiration(now) as u32, Ordering::Relaxed);
            });
        }
    }

    fn set_alarm(&self, cs: &CriticalSection, at: u64) -> bool {
        self.next.store(at as u32, Ordering::Relaxed);
        true
    }
}

impl Driver for MyDriver {
    fn now(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed) as u64
    }

    fn schedule_wake(&self, at: u64, waker: &Waker) {
        critical_section::with(|cs| {
            let mut queue = self.queue.borrow(cs).borrow_mut();
            if queue.schedule_wake(at, waker) {
                let mut next = queue.next_expiration(self.now());
                while !self.set_alarm(&cs, next) {
                    next = queue.next_expiration(self.now());
                }
            }
        });
    }
}

embassy_time_driver::time_driver_impl!(static DRIVER: MyDriver = MyDriver::new());

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    //let dp = stm32f4xx_hal::pac::Peripherals::take().unwrap();

    let cp = cortex_m::Peripherals::take().unwrap();

    let clocks = 48_000_000; // setup_clocks(dp.RCC);

    // Set PB7 to push-pull output
    pac::GPIOB
        .moder()
        .modify(|w| w.set_moder(7, pac::gpio::vals::Moder::OUTPUT));
    pac::GPIOB
        .otyper()
        .modify(|w| w.set_ot(7, pac::gpio::vals::Ot::PUSH_PULL));
    pac::GPIOB
        .ospeedr()
        .modify(|w| w.set_ospeedr(7, pac::gpio::vals::Ospeedr::HIGH_SPEED));
    pac::GPIOB
        .pupdr()
        .modify(|w| w.set_pupdr(7, pac::gpio::vals::Pupdr::FLOATING));

    // SAFETY: PB7 is not used anywhere else
    let led = unsafe { Output::new(PB7::steal(), Level::High, Speed::Low) };

    // Setup SysTick for 1 kHz ticks (1ms)
    let mut syst = cp.SYST;
    syst.set_clock_source(SystClkSource::Core);
    syst.set_reload(clocks / 1000 - 1);
    syst.clear_current();
    syst.enable_counter();
    syst.enable_interrupt();

    blink_loop(led).await;
}

async fn blink_loop(mut led: Output<'static>) {
    loop {
        led.set_high();
        Timer::after_millis(300).await;
        led.set_low();
        Timer::after_millis(300).await;
    }
}

#[cortex_m_rt::exception]
fn SysTick() {
    critical_section::with(|cs| {
        // Call the driver's tick handler
        DRIVER.on_tick();
    });
}
