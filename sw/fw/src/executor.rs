//! Multicore-aware thread-mode embassy executor.

use core::marker::PhantomData;

use embassy_executor::{raw, Spawner};
use embassy_time::Instant;
use esp_hal::get_core;
use portable_atomic::{AtomicBool, AtomicU64, Ordering};

pub(crate) const THREAD_MODE_CONTEXT: u8 = 16;

static SIGNAL_WORK_THREAD_MODE: [AtomicBool; 1] = [AtomicBool::new(false)];

#[export_name = "__pender"]
fn __pender(context: *mut ()) {
    use esp_hal::system::SoftwareInterrupt;

    let context = (context as usize).to_le_bytes();

    match context[0] {
        // For interrupt executors, the context value is the
        // software interrupt number
        0 => unsafe { SoftwareInterrupt::<0>::steal().raise() },
        1 => unsafe { SoftwareInterrupt::<1>::steal().raise() },
        2 => unsafe { SoftwareInterrupt::<2>::steal().raise() },
        3 => unsafe { SoftwareInterrupt::<3>::steal().raise() },
        other => {
            assert_eq!(other, THREAD_MODE_CONTEXT);
            // THREAD_MODE_CONTEXT id is reserved for thread mode executors
            pend_thread_mode(context[1] as usize)
        }
    }
}

pub(crate) fn pend_thread_mode(core: usize) {
    // Signal that there is work to be done.
    SIGNAL_WORK_THREAD_MODE[core].store(true, Ordering::SeqCst);
}

/// A thread aware Executor
pub struct Executor {
    inner: raw::Executor,
    not_send: PhantomData<*mut ()>,
    sleep_ticks: AtomicU64,
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
            sleep_ticks: AtomicU64::new(0),
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
                // duration.as_ticks()
            }
        });
        // if an interrupt occurred while waiting, it will be serviced
        // here
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}
