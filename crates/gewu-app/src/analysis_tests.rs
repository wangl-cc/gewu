use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Barrier,
    },
    thread,
    time::Duration,
};

use super::Slots;

#[test]
fn concurrent_readers_share_success_and_retry_after_a_failure() {
    let slots = Slots::default();
    assert_eq!(slots.read(0, 32, || Err("failed")), Err("failed"));
    let start = Barrier::new(2);
    let calls = AtomicUsize::new(0);
    let read = || {
        start.wait();
        slots
            .read(0, 32, || {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok::<_, &str>(37)
            })
            .unwrap()
    };
    let (a, b) = thread::scope(|scope| {
        let first = scope.spawn(read);
        (read(), first.join().unwrap())
    });
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(Arc::ptr_eq(&a, &b));
}

#[test]
fn an_obsolete_calculation_cannot_evict_a_new_parameter_set() {
    let slots = Slots::default();
    let (started, running) = mpsc::channel();
    let (finish, pending) = mpsc::channel();
    thread::scope(|scope| {
        let slots = &slots;
        let old = scope.spawn(move || {
            slots
                .read(0, 16, || {
                    started.send(()).unwrap();
                    pending.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok::<_, ()>(16)
                })
                .unwrap()
        });
        running.recv_timeout(Duration::from_secs(5)).unwrap();
        let current = slots.read(0, 64, || Ok::<_, ()>(64)).unwrap();
        finish.send(()).unwrap();
        assert_eq!(*old.join().unwrap(), 16);
        let cached = slots
            .read(0, 64, || Err::<i32, _>("unexpected cache miss"))
            .unwrap();
        assert!(Arc::ptr_eq(&cached, &current));
    });
}
