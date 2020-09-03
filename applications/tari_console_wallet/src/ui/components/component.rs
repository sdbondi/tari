use crate::ui::state::AppState;
use tui::{backend::Backend, layout::Rect, Frame};

pub trait Component<B: Backend> {
    fn draw(&mut self, f: &mut Frame<B>, area: Rect, app_state: &AppState);

    fn on_key(&mut self, app_state: &mut AppState, c: char) {}

    fn on_up(&mut self, app_state: &mut AppState) {}

    fn on_down(&mut self, app_state: &mut AppState) {}

    fn on_esc(&mut self, app_state: &mut AppState) {}
    fn on_backspace(&mut self, app_state: &mut AppState) {}
}
