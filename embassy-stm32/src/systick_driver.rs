use core::{
    cell::{Cell, RefCell},
    cmp::min,
    sync::atomic::{AtomicU32, Ordering},
    task::Waker,
};

use cortex_m::peripheral::syst::SystClkSource;
use cortex_m_rt::exception;
use critical_section::CriticalSection;
use embassy_executor::raw::TaskRef;
use crate::{rcc, time::Hertz};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{TickType, TICK_HZ};
use embassy_time_driver::Driver;

// Use half of the TickType range as past timestamps and the other part the future ones
// Describe the sign/unsigned limit: [0, TICK_DIFF_LIMIT[ is a positive result (past events), otherwise negative (future events)
const TICK_DIFF_LIMIT: TickType = (TickType::MAX / 2) + 1;

fn is_expired(now: TickType, deadline: TickType) -> bool {
    now.wrapping_sub(deadline) < TICK_DIFF_LIMIT // Is expired if the result is non negative
}

struct Queue {
    head: Cell<Option<TaskRef>>,
}

impl Queue {
    /// Creates a new timer queue.
    pub const fn new() -> Self {
        Self { head: Cell::new(None) }
    }

    /// Schedules a task to run at a specific time.
    ///
    /// If this function returns `true`, the called should find the next expiration time and set
    /// a new alarm for that time.
    pub fn schedule_wake(&mut self, at: TickType, waker: &Waker) -> bool {
        let task = embassy_executor::raw::task_from_waker(waker);
        let item = task.timer_queue_item();
        if item.next.get().is_none() {
            // If not in the queue, add it and update.
            let prev = self.head.replace(Some(task));
            item.next.set(if prev.is_none() {
                Some(unsafe { TaskRef::dangling() })
            } else {
                prev
            });
            item.expires_at.set(at);
            true
        } else if is_expired(item.expires_at.get(), at) {
            // If expiration is sooner than previously set, update.
            item.expires_at.set(at);
            true
        } else {
            // Task does not need to be updated.
            false
        }
    }

    /// Dequeues expired timers and returns the next alarm time.
    ///
    /// The provided callback will be called for each expired task. Tasks that never expire
    /// will be removed, but the callback will not be called.
    pub fn next_expiration(&mut self, now: TickType) -> TickType {
        let mut delta = TICK_DIFF_LIMIT; // Put the delta the farthest possible

        self.retain(|p| {
            let item = p.timer_queue_item();
            let expires = item.expires_at.get();

            if is_expired(now, expires) {
                // Timer expired, process task.
                embassy_executor::raw::wake_task(p);
                false
            } else {
                // Timer didn't yet expire, or never expires (expires >= now).
                delta = min(delta, expires - now);
                true
            }
        });

        delta + now
    }

    fn retain(&self, mut f: impl FnMut(TaskRef) -> bool) {
        let mut prev = &self.head;
        while let Some(p) = prev.get() {
            if unsafe { p == TaskRef::dangling() } {
                // prev was the last item, stop
                break;
            }
            let item = p.timer_queue_item();
            if f(p) {
                // Skip to next
                prev = &item.next;
            } else {
                // Remove it
                prev.set(item.next.get());
                item.next.set(None);
            }
        }
    }
}

trait AtomicTickImpl {
    type Atom;
    type Const;

    fn load(atom: &Self::Atom, order: Ordering) -> TickType;
    fn increment(atom: &Self::Atom, order: Ordering) -> TickType;
    fn store(atom: &Self::Atom, val: TickType, order: Ordering);
}

struct AtomicTickImplU32 {}
impl AtomicTickImplU32 {
    pub const fn new(val: TickType) -> AtomicU32 {
        AtomicU32::new(val as u32)
    }
}

impl AtomicTickImpl for u32 {
    type Atom = AtomicU32;
    type Const = AtomicTickImplU32;

    fn load(atom: &Self::Atom, order: Ordering) -> TickType {
        atom.load(order) as TickType
    }

    fn increment(atom: &Self::Atom, order: Ordering) -> TickType {
        atom.fetch_add(1, order) as TickType
    }

    fn store(atom: &Self::Atom, val: TickType, order: Ordering) {
        atom.store(val as u32, order);
    }
}

#[cfg(target_has_atomic = "64")]
struct AtomicTickImplU64 {}
#[cfg(target_has_atomic = "64")]
impl AtomicTickImplU64 {
    pub const fn new(val: TickType) -> AtomicU64 {
        AtomicU64::new(val as u64)
    }
}

#[cfg(target_has_atomic = "64")]
impl AtomicTickImpl for u64 {
    type Atom = AtomicU64;
    type Const = AtomicTickImplU64;

    fn load(atom: &Self::Atom, order: Ordering) -> TickType {
        atom.load(order) as TickType
    }

    fn increment(atom: &Self::Atom, order: Ordering) -> TickType {
        atom.fetch_add(1, order) as TickType
    }

    fn store(atom: &Self::Atom, val: TickType, order: Ordering) {
        atom.store(val, order);
    }
}

struct AtomicTickImplU64Mutex {}
impl AtomicTickImplU64Mutex {
    pub const fn new(val: TickType) -> Mutex<CriticalSectionRawMutex, RefCell<TickType>> {
        Mutex::const_new(CriticalSectionRawMutex::new(), RefCell::new(val))
    }
}

#[cfg(not(target_has_atomic = "64"))]
impl AtomicTickImpl for u64 {
    type Atom = Mutex<CriticalSectionRawMutex, RefCell<TickType>>;
    type Const = AtomicTickImplU64Mutex;


    fn load(atom: &Self::Atom, _order: Ordering) -> TickType {
        atom.lock(|data| *data.borrow())
    }

    fn increment(atom: &Self::Atom, _order: Ordering) -> TickType {
        atom.lock(|data| {
            let old = *data.borrow_mut();
            *data.borrow_mut() = old.wrapping_add(1);
            old
        })
    }

    fn store(atom: &Self::Atom, val: TickType, _order: Ordering) {
        atom.lock(|data| {
            *data.borrow_mut() = val;
        });
    }
}

pub struct AtomicTick {
    inner: <TickType as AtomicTickImpl>::Atom,
}

impl AtomicTick {
    const fn const_new(val: TickType) -> Self {
        Self {
            inner: <TickType as AtomicTickImpl>::Const::new(val),
        }
    }

    fn load(&self, order: Ordering) -> TickType {
        <TickType as AtomicTickImpl>::load(&self.inner, order)
    }

    fn increment(&self, order: Ordering) -> TickType {
        <TickType as AtomicTickImpl>::increment(&self.inner, order)
    }

    fn store(&self, val: TickType, order: Ordering) {
        <TickType as AtomicTickImpl>::store(&self.inner, val, order);
    }
}

struct SystickDriver {
    current: AtomicTick,
    next: AtomicTick,
    queue: Mutex<CriticalSectionRawMutex, RefCell<Queue>>,
}

impl SystickDriver {
    fn init(&self) {
        let mut syst = unsafe { cortex_m::Peripherals::steal().SYST };

        // Set the SYSTICK
        syst.set_clock_source(SystClkSource::External);
        let frequency = unsafe {
            let hclk1: Hertz = rcc::get_freqs().hclk1.to_hertz().unwrap();

            if syst.get_clock_source() == SystClkSource::External {
                hclk1 / 8u32
            } else {
                hclk1
            }
        };
        let reload = frequency / Hertz(TICK_HZ as u32) - 1;
        syst.set_reload(reload);
        syst.clear_current();

        // Start the SYSTICK
        syst.enable_counter();
        syst.enable_interrupt();
    }

    fn on_interrupt(&self) {
        let now = self.current.increment(Ordering::Relaxed) + 1;
        let next = self.next.load(Ordering::Relaxed);
        if is_expired(now, next) {
            critical_section::with(|cs| {
                let mut queue = self.queue.borrow(cs).borrow_mut();
                let next = queue.next_expiration(now);
                self.next.store(next, Ordering::Relaxed);
            });
        }
    }

    fn set_alarm(&self, _cs: &CriticalSection, at: TickType) -> bool {
        if is_expired(self.now(), at) {
            return false;
        }
        self.next.store(at, Ordering::Relaxed);
        true
    }
}

impl Driver for SystickDriver {
    fn now(&self) -> TickType {
        self.current.load(Ordering::Relaxed) as TickType
    }

    fn schedule_wake(&self, at: TickType, waker: &Waker) {
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

embassy_time_driver::time_driver_impl!(static SYSTICK_DRIVER: SystickDriver = SystickDriver {
    current: AtomicTick::const_new(0),
    next: AtomicTick::const_new(0),
    queue:  Mutex::const_new(CriticalSectionRawMutex::new(), RefCell::new(Queue::new())),
});

#[exception]
fn SysTick() {
    // Call the driver's tick handler
    SYSTICK_DRIVER.on_interrupt();
}

pub fn init(_cs: critical_section::CriticalSection) {
    SYSTICK_DRIVER.init()
}

