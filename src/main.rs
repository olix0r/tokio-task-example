extern crate futures;
#[macro_use]
extern crate log;
extern crate rand;
extern crate pretty_env_logger;
extern crate tokio_timer;

use futures::{Future, Poll, Async};
use futures::task::{self, EventSet, UnparkEvent};
use rand::Rng;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_timer::{Timer, Sleep};

const UNPARK_AWARE: bool = true;
const SIZE: usize = 1_000_000;
const MIN_SLEEP_MS: u64 = 10;
const MAX_SLEEP_MS: u64 = 60_000;
const SLEEP_GRANULARITY_MS: u64 = 10;

fn main() {
    drop(pretty_env_logger::init());

    let poll_time = if UNPARK_AWARE {
        println!("unparking events={} max={}ms granularity={}ms",
                 SIZE,
                 MAX_SLEEP_MS,
                 SLEEP_GRANULARITY_MS);
        let f = unparking();
        info!("running");
        f.wait().unwrap()
    } else {
        println!("iterating events={} max={}ms granularity={}ms",
                 SIZE,
                 MAX_SLEEP_MS,
                 SLEEP_GRANULARITY_MS);
        let f = iterating();
        info!("running");
        f.wait().unwrap()
    };
    println!("poll time {}.{}",
             poll_time.as_secs(),
             poll_time.subsec_nanos());
}

/// A helper for sleeping a random amount of time.
struct RandSleep<R> {
    timer: Timer,
    rng: R,
}
impl Default for RandSleep<rand::ThreadRng> {
    fn default() -> RandSleep<rand::ThreadRng> {
        RandSleep {
            timer: Timer::default(),
            rng: rand::thread_rng(),
        }
    }
}
impl<R: Rng> RandSleep<R> {
    pub fn sleep(&mut self) -> Sleep {
        let ms: u64 = self.rng.gen_range(MIN_SLEEP_MS, MAX_SLEEP_MS) / SLEEP_GRANULARITY_MS *
                      SLEEP_GRANULARITY_MS;
        self.timer.sleep(Duration::from_millis(ms))
    }
}

/// Polls all pending futures when polled.
fn iterating() -> IteratingSleeper {
    let mut timer = RandSleep::default();
    let mut sleeps = VecDeque::new();
    for _ in 0..SIZE {
        sleeps.push_back(timer.sleep());
    }
    IteratingSleeper {
        sleeps: sleeps,
        poll_time: Duration::default(),
    }
}
struct IteratingSleeper {
    sleeps: VecDeque<Sleep>,
    poll_time: Duration,
}
impl Future for IteratingSleeper {
    type Item = Duration;
    type Error = ();
    fn poll(&mut self) -> Poll<Duration, ()> {
        let start = Instant::now();
        debug!("poll() sleeps={}", self.sleeps.len());
        let mut all_none = true;
        for _ in 0..self.sleeps.len() {
            let mut sleep = self.sleeps.pop_front().take().unwrap();
            trace!("polling");
            if sleep.poll()? == Async::NotReady {
                all_none = false;
                self.sleeps.push_back(sleep);
            }
        }

        self.poll_time += start.elapsed();
        debug!("poll() done={}", all_none);
        if all_none {
            Ok(Async::Ready(self.poll_time))
        } else {
            Ok(Async::NotReady)
        }
    }
}

/// Polls only necessary futures when polled.
fn unparking() -> UnparkingSleeper {
    let mut timer = RandSleep::default();
    // When generating the list of sleeps, we configure the `ready` set to include all
    // items so that the unpark contexts are initialized on the first poll.
    let mut sleeps = Vec::with_capacity(SIZE);
    let mut ready = Vec::with_capacity(SIZE);
    for i in 0..SIZE {
        sleeps.push(Some(timer.sleep()));
        ready.push(i);
    }
    UnparkingSleeper {
        sleeps: sleeps,
        n_active: SIZE,
        ready: Arc::new(Ready(Mutex::new(ready))),
        poll_time: Duration::default(),
    }
}
struct UnparkingSleeper {
    sleeps: Vec<Option<Sleep>>,
    /// The number of actively pending sleeps remaining.
    n_active: usize,
    /// An EventSet of sleep indexes that should be checked on next poll.
    ready: Arc<Ready>,
    poll_time: Duration,
}
impl Future for UnparkingSleeper {
    type Item = Duration;
    type Error = ();
    fn poll(&mut self) -> Poll<Duration, ()> {
        let start = Instant::now();

        // Grab the EventSet lock and drain the current ready indexes.
        let ready: Vec<usize> = {
            let mut set = self.ready.0.lock().unwrap();
            debug!("poll() active={} ready={}", self.n_active, set.len());
            let drain = set.drain(..).collect();
            drop(set);
            drain
        };

        // Check all of the ready indexes.
        for idx in ready {
            if let Some(mut sleep) = self.sleeps[idx].take() {
                trace!("polling {}", idx);
                // Set the unpark event so that `ready` is updated with the proper index
                // when this future is satisfied.
                let event = UnparkEvent::new(self.ready.clone(), idx);
                match task::with_unpark_event(event, || sleep.poll())? {
                    Async::NotReady => {
                        trace!("not ready {}", idx);
                        self.sleeps[idx] = Some(sleep);
                    }
                    Async::Ready(_) => {
                        trace!("ready {}", idx);
                        self.n_active -= 1;
                    }
                }
            }
        }

        self.poll_time += start.elapsed();
        debug!("poll() active={}", self.n_active);
        if self.n_active == 0 {
            Ok(Async::Ready(self.poll_time))
        } else {
            Ok(Async::NotReady)
        }
    }
}

/// Tracks ready events.
///
/// We naively use a `Mutex` here. In a real application, this should be some sort of
/// concurrent data structure (like `futures::Stack`).
struct Ready(Mutex<Vec<usize>>);
impl EventSet for Ready {
    fn insert(&self, id: usize) {
        trace!("inserting event {}", id);
        let mut ready = self.0.lock().unwrap();
        ready.push(id);
    }
}
