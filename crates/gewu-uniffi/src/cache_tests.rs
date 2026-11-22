use super::*;

#[test]
fn a_slot_keeps_one_result_and_recomputes_for_new_parameters() {
    let slots = Slots::<u8, u32, u32>::default();
    let mut computed = 0;
    let mut get = |params| {
        slots
            .get_or_compute(0, params, || {
                computed += 1;
                Ok::<_, ()>(params * 10)
            })
            .unwrap()
    };
    assert_eq!([get(1), get(1), get(2), get(1)], [10, 10, 20, 10]);
    assert_eq!(computed, 3);
    assert_eq!(slots.lock().len(), 1);
    assert!(slots.get_or_compute(1, 0, || Err(())).is_err());
    assert_eq!(slots.lock().len(), 1, "failures are not kept");
}
