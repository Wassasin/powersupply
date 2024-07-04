//! Multicore-aware thread-mode embassy executor.

use core::marker::PhantomData;

use embassy_executor::{raw, Spawner};
use embassy_time::Instant;
use esp_hal::get_core;
use portable_atomic::{AtomicBool, AtomicU64, Ordering};

pub(crate) const THREAD_MODE_CONTEXT: u8 = 16;

static SIGNAL_WORK_THREAD_MODE: [AtomicBool; 1] = [AtomicBool::new(false)];

pub(crate) fn pend_thread_mode(core: usize) {
    // Signal that there is work to be done.
    SIGNAL_WORK_THREAD_MODE[core].store(true, Ordering::SeqCst);
}

static SLEEP_COUNT_FROM: AtomicU64 = AtomicU64::new(0);
static SLEEP_TOTAL_TICKS: AtomicU64 = AtomicU64::new(0);

pub struct SleepStats {
    sleep: u64,
    total: u64,
}

impl SleepStats {
    pub fn current_restart() -> Self {
        critical_section::with(|_| {
            let sleep = SLEEP_TOTAL_TICKS.swap(0, Ordering::Relaxed);
            let from = SLEEP_COUNT_FROM.swap(Instant::now().as_ticks(), Ordering::Relaxed);
            let total = Instant::now().as_ticks() - from;
            Self { sleep, total }
        })
    }

    pub fn as_permille(&self) -> u64 {
        self.sleep * 1000 / self.total
    }
}

/// A thread aware Executor
pub struct Executor {
    inner: raw::Executor,
    not_send: PhantomData<*mut ()>,
}

impl Executor {
    /// Create a new Executor.
    pub fn new() -> Self {
        Self {
            inner: raw::Executor::new(usize::from_le_bytes([
                THREAD_MODE_CONTEXT,
                get_core() as u8,
                0,
                0,
            ]) as *mut ()),
            not_send: PhantomData,
        }
    }

    /// Run the executor.
    ///
    /// The `init` closure is called with a [`Spawner`] that spawns tasks on
    /// this executor. Use it to spawn the initial task(s). After `init`
    /// returns, the executor starts running the tasks.
    ///
    /// To spawn more tasks later, you may keep copies of the [`Spawner`] (it is
    /// `Copy`), for example by passing it as an argument to the initial
    /// tasks.
    ///
    /// This function requires `&'static mut self`. This means you have to store
    /// the Executor instance in a place where it'll live forever and grants
    /// you mutable access. There's a few ways to do this:
    ///
    /// - a [StaticCell](https://docs.rs/static_cell/latest/static_cell/) (safe)
    /// - a `static mut` (unsafe)
    /// - a local variable in a function you know never returns (like `fn main()
    ///   -> !`), upgrading its lifetime with `transmute`. (unsafe)
    ///
    /// This function never returns.
    pub fn run(&'static mut self, init: impl FnOnce(Spawner)) -> ! {
        init(self.inner.spawner());

        let cpu = get_core() as usize;

        loop {
            unsafe {
                self.inner.poll();
                self.wait_impl(cpu);
            }
        }
    }

    fn wait_impl(&'static self, cpu: usize) {
        // we do not care about race conditions between the load and store operations,
        // interrupts will only set this value to true.
        critical_section::with(|_| {
            // if there is work to do, loop back to polling
            // TODO can we relax this?
            if SIGNAL_WORK_THREAD_MODE[cpu].load(Ordering::SeqCst) {
                SIGNAL_WORK_THREAD_MODE[cpu].store(false, Ordering::SeqCst);
            }
            // if not, wait for interrupt
            else {
                let start = Instant::now();
                unsafe { core::arch::asm!("wfi") };
                let duration = start.elapsed();
                SLEEP_TOTAL_TICKS.fetch_add(duration.as_ticks(), Ordering::Relaxed);
            }
        });
        // if an interrupt occurred while waiting, it will be serviced here
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}
