use crate::{
    GLOBAL_THREAD_TIMINGS, PlatformDispatcher, Priority, PriorityQueueSender, RealtimePriority,
    RunnableVariant, THREAD_TIMINGS, ThreadTaskTimings,
};
use priority_threadpool::ThreadPool;
use std::thread::ThreadId;

pub struct Dispatcher {
    main_thread_id: ThreadId,
    main_tx: PriorityQueueSender<RunnableVariant>,
    threadpool: ThreadPool<Priority>,
}

impl PlatformDispatcher for Dispatcher {
    fn get_all_timings(&self) -> Vec<crate::ThreadTaskTimings> {
        let global_thread_timings = GLOBAL_THREAD_TIMINGS.lock();
        ThreadTaskTimings::convert(&global_thread_timings)
    }

    fn get_current_thread_timings(&self) -> Vec<crate::TaskTiming> {
        THREAD_TIMINGS.with(|timings| {
            let timings = timings.lock();
            let timings = &timings.timings;

            let mut vec = Vec::with_capacity(timings.len());

            let (s1, s2) = timings.as_slices();

            vec.extend_from_slice(s1);
            vec.extend_from_slice(s2);

            vec
        })
    }

    fn is_main_thread(&self) -> bool {
        std::thread::current().id() == self.main_thread_id
    }

    fn dispatch(
        &self,
        runnable: RunnableVariant,
        _label: Option<crate::TaskLabel>,
        priority: Priority,
    ) {
        // TODO(mdeand): Unify the types?
        let runnable = match runnable {
            RunnableVariant::Meta(_runnable) => unimplemented!(),
            RunnableVariant::Compat(runnable) => runnable,
        };

        self.threadpool.queue(&priority, runnable);
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        match self.main_tx.send(priority, runnable) {
            Ok(_) => {}
            Err(runnable) => {
                // TODO(mdeand): Comment - Why do we forget this?
                std::mem::forget(runnable);
            }
        }
    }

    fn dispatch_after(&self, duration: std::time::Duration, runnable: RunnableVariant) {
        // TODO(mdeand): Unify the types?
        let runnable = match runnable {
            RunnableVariant::Meta(_runnable) => unimplemented!(),
            RunnableVariant::Compat(runnable) => runnable,
        };

        self.threadpool
            .queue_delayed(&Priority::Low, duration, runnable);
    }

    fn spawn_realtime(&self, _priority: RealtimePriority, f: Box<dyn FnOnce() + Send>) {
        // TODO(mdeand): There's a crate (thread-priority) that implements thread
        // TODO(mdeand): priorities, but I don't want to add it right now.

        std::thread::spawn(move || {
            f();
        });
    }
}

impl priority_threadpool::Priority for Priority {
    const COUNT: usize = 3;

    fn index(&self) -> usize {
        match self {
            Priority::High => 0,
            Priority::Medium => 1,
            Priority::Low => 2,
            _ => unreachable!(),
        }
    }
}
