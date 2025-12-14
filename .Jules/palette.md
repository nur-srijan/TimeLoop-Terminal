## 2024-03-20 - [Micro-UX: Tooltips in egui]
**Learning:** In egui, adding tooltips is as simple as chaining `.on_hover_text()` to the widget response. This is a high-impact, low-effort way to improve discoverability for icon-less or context-heavy buttons.
**Action:** Always check `egui` widgets for `Response` objects that can accept hover interactions.
