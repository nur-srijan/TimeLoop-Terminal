## 2024-03-21 - [Interactive Timelines in egui]
**Learning:** Making a progress bar interactive (scrubbable) in `egui` requires changing the sense to `egui::Sense::click_and_drag()` and manually calculating the new value from `response.interact_pointer_pos()`. This significantly improves usability for time-based data.
**Action:** When visualizing progress or timelines, always consider if the user might want to control it, not just view it.
