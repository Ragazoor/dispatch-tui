//! Cursor motion shared by the agent tree's list sections.

/// Up/down/top/bottom/half-page motion over a list whose rows run
/// `0..=last()`. Implementors supply the cursor, the last row and the
/// viewport height; the motions are written once here.
pub trait ListCursor {
    fn cursor_mut(&mut self) -> &mut usize;

    /// The last row's index.
    fn last(&self) -> usize;

    /// Rows the pane shows, which sets the half-page step.
    fn viewport_rows(&self) -> usize;

    fn up(&mut self) {
        let cursor = self.cursor_mut();
        *cursor = cursor.saturating_sub(1);
    }

    fn down(&mut self) {
        let last = self.last();
        let cursor = self.cursor_mut();
        *cursor = (*cursor + 1).min(last);
    }

    fn top(&mut self) {
        *self.cursor_mut() = 0;
    }

    fn bottom(&mut self) {
        *self.cursor_mut() = self.last();
    }

    fn half_page_down(&mut self) {
        let (last, step) = (
            self.last(),
            crate::agent_tree::pane::half_page(self.viewport_rows()),
        );
        let cursor = self.cursor_mut();
        *cursor = (*cursor + step).min(last);
    }

    fn half_page_up(&mut self) {
        let step = crate::agent_tree::pane::half_page(self.viewport_rows());
        let cursor = self.cursor_mut();
        *cursor = cursor.saturating_sub(step);
    }
}

#[cfg(test)]
mod tests {
    use super::ListCursor;

    struct List {
        cursor: usize,
        last: usize,
        viewport_rows: usize,
    }

    impl ListCursor for List {
        fn cursor_mut(&mut self) -> &mut usize {
            &mut self.cursor
        }
        fn last(&self) -> usize {
            self.last
        }
        fn viewport_rows(&self) -> usize {
            self.viewport_rows
        }
    }

    fn list(cursor: usize) -> List {
        List {
            cursor,
            last: 9,
            viewport_rows: 6,
        }
    }

    #[test]
    fn up_and_down_stop_at_the_ends() {
        let mut l = list(0);
        l.up();
        assert_eq!(l.cursor, 0);
        l.down();
        assert_eq!(l.cursor, 1);
        let mut l = list(9);
        l.down();
        assert_eq!(l.cursor, 9);
    }

    #[test]
    fn top_and_bottom_jump_to_the_ends() {
        let mut l = list(4);
        l.bottom();
        assert_eq!(l.cursor, 9);
        l.top();
        assert_eq!(l.cursor, 0);
    }

    #[test]
    fn half_page_moves_half_the_viewport_and_clamps() {
        let mut l = list(4);
        l.half_page_down();
        assert_eq!(l.cursor, 7);
        l.half_page_down();
        assert_eq!(l.cursor, 9);
        l.half_page_up();
        assert_eq!(l.cursor, 6);
        let mut l = list(1);
        l.half_page_up();
        assert_eq!(l.cursor, 0);
    }
}
