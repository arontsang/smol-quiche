use std::future::Future;

use async_executor::{LocalExecutor, Task};
use futures_lite::{future, prelude::*};

/// Task priority.
#[repr(usize)]
#[derive(Debug, Clone, Copy)]
pub enum Priority {
    High = 0,
    Low = 1,
}

pub struct PriorityExecutor<'a> {
    ex: [LocalExecutor<'a>; 2],
}

impl<'a> PriorityExecutor<'a> {
    /// Creates a new executor.
    pub const fn new() -> PriorityExecutor<'a> {
        PriorityExecutor {
            ex: [LocalExecutor::new(), LocalExecutor::new()],
        }
    }

    /// Spawns a task with the given priority.
    pub fn spawn<T: 'a>(
        &self,
        priority: Priority,
        future: impl Future<Output = T> + 'a,
    ) -> Task<T> {
        self.ex[priority as usize].spawn(future)
    }

    /// Runs the executor forever.
    pub async fn run(&self) {
        loop {
            for _ in 0..200 {
                let t0 = self.ex[0].tick();
                let t1 = self.ex[1].tick();

                // Wait until one of the ticks completes, trying them in order from highest
                // priority to lowest priority.
                t0.or(t1).await;
            }

            // Yield every now and then.
            future::yield_now().await;
        }
    }
}

pub fn auto_reset_event() -> (RaiseEvent, Waiter) {
    let (sender, receiver) = smol::channel::bounded::<()>(1);
    (RaiseEvent{ resetter: sender }, Waiter{ waiter: receiver })
}

pub struct Waiter {
    waiter: smol::channel::Receiver::<()>
}


pub struct RaiseEvent {
    resetter: smol::channel::Sender::<()>
}

impl RaiseEvent {
    pub fn reset(&self) -> () {
        match self.resetter.try_send(()) {
            Ok(()) => (),
            Err(_) => (),
        }
    }
}

impl Waiter {
    pub async fn wait_once(&self) -> () {
        match self.waiter.recv().await {
            Ok(()) => (),
            Err(_) => (),
        }
    }
}

impl Clone for RaiseEvent {
    
    fn clone(&self) -> Self { 
        RaiseEvent{ resetter: self.resetter.clone()}
     }
}