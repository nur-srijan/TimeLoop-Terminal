## 2024-03-21 - [Interactive Timelines]
**Learning:** Users instinctively expect progress bars to be scrubbable. Converting a passive `egui::Sense::hover()` rect to `egui::Sense::click_and_drag()` transforms a static indicator into a powerful navigation tool with minimal code changes.
**Action:** When visualizing progress or duration, always consider if the user might want to control it directly.

## 2024-03-21 - [Keyboard Shortcuts in egui]
**Learning:** Adding global shortcuts like Spacebar for Play/Pause in `egui` requires checking `ctx.input()` but crucially also `!ctx.wants_keyboard_input()` to avoid triggering actions while typing in input fields.
**Action:** Always guard global shortcuts with `!ctx.wants_keyboard_input()` to prevent UX conflicts.
