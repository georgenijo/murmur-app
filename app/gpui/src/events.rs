//! Local GPUI entity events, separate from Tauri's window transport.
//! Keep this typed UFCS call: the repository transport guard permits only
//! this spelling here, and still rejects every Tauri broadcast API.
pub(crate) fn publish<T: gpui::EventEmitter<E> + 'static, E: 'static>(
    cx: &mut gpui::Context<T>,
    event: E,
) {
    gpui::Context::<T>::emit(cx, event);
}
