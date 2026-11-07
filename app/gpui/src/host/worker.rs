//! Worker dispatch is the first operation of the executable, before creating
//! an Application, NSApplication, runtime-shader/Metal state, or a window.
pub trait Workers {
    fn diarization(&self) -> Option<i32>;
    fn coreml_install(&self) -> Option<i32>;
    fn capture_agent(&self) -> Option<i32>;
    fn capture_helper(&self) -> Option<i32>;
}
pub fn dispatch(workers: &dyn Workers) -> Option<i32> {
    workers
        .diarization()
        .or_else(|| workers.coreml_install())
        .or_else(|| workers.capture_agent())
        .or_else(|| workers.capture_helper())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct Probe {
        calls: RefCell<Vec<u8>>,
        selected: u8,
    }
    impl Probe {
        fn call(&self, i: u8) -> Option<i32> {
            self.calls.borrow_mut().push(i);
            (i == self.selected).then_some(17)
        }
    }
    impl Workers for Probe {
        fn diarization(&self) -> Option<i32> {
            self.call(0)
        }
        fn coreml_install(&self) -> Option<i32> {
            self.call(1)
        }
        fn capture_agent(&self) -> Option<i32> {
            self.call(2)
        }
        fn capture_helper(&self) -> Option<i32> {
            self.call(3)
        }
    }
    #[test]
    fn dispatch_short_circuits_in_shipping_order() {
        for selected in 0..=4 {
            let p = Probe {
                calls: RefCell::default(),
                selected,
            };
            assert_eq!(dispatch(&p), (selected < 4).then_some(17));
            assert_eq!(*p.calls.borrow(), (0..=selected.min(3)).collect::<Vec<_>>());
        }
    }
}
