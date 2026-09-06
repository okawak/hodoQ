//! Month and agenda calendar rendering.
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{
    IconName, Selectable as _, Sizable as _, button::Button, scroll::ScrollableElement as _,
};
use time::{Date, OffsetDateTime, UtcOffset};

use crate::domain::{Due, Task, TaskStatus};

use super::theme;

use super::due::{
    calendar_leading_days, due_date, format_due_display, format_due_input, shift_month,
};
use super::{
    CALENDAR_DAY_CELL_HEIGHT, CALENDAR_GRID_MIN_HEIGHT, CALENDAR_WEEKDAY_HEIGHT, CalendarMode,
    Workspace, priority_color, section_label,
};

impl Workspace {
    pub(super) fn render_calendar(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .pt_4()
            .when(self.calendar_mode == CalendarMode::Month, |header| {
                header
                    .child({
                        let entity = cx.entity();
                        Button::new("calendar-previous")
                            .debug_selector(|| "calendar-previous".to_owned())
                            .small()
                            .icon(IconName::ArrowLeft)
                            .tooltip("前月")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.calendar_month = shift_month(this.calendar_month, -1);
                                    cx.notify();
                                });
                            })
                    })
                    .child(
                        div()
                            .debug_selector(|| "calendar-month-label".to_owned())
                            // Reserve space for two-digit months so navigation never shifts.
                            .w(gpui::rems(8.0))
                            .flex_shrink_0()
                            .text_center()
                            .font_weight(FontWeight::BOLD)
                            .child(format!(
                                "{}年{}月",
                                self.calendar_month.year(),
                                self.calendar_month.month() as u8
                            )),
                    )
                    .child({
                        let entity = cx.entity();
                        Button::new("calendar-next")
                            .debug_selector(|| "calendar-next".to_owned())
                            .small()
                            .icon(IconName::ArrowRight)
                            .tooltip("翌月")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.calendar_month = shift_month(this.calendar_month, 1);
                                    cx.notify();
                                });
                            })
                    })
                    .child({
                        let entity = cx.entity();
                        Button::new("calendar-current-month")
                            .debug_selector(|| "calendar-current-month".to_owned())
                            .small()
                            .label("今月")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    let now = OffsetDateTime::now_utc();
                                    let offset =
                                        UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
                                    let today = now.to_offset(offset).date();
                                    this.calendar_month =
                                        Date::from_calendar_date(today.year(), today.month(), 1)
                                            .unwrap_or(today);
                                    cx.notify();
                                });
                            })
                    })
            })
            .when(self.calendar_mode == CalendarMode::Agenda, |header| {
                header.child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .child("アジェンダ"),
                )
            })
            .child(div().flex_1())
            .child({
                let entity = cx.entity();
                Button::new("calendar-month")
                    .small()
                    .label("月")
                    .selected(self.calendar_mode == CalendarMode::Month)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.calendar_mode = CalendarMode::Month;
                            cx.notify();
                        });
                    })
            })
            .child({
                let entity = cx.entity();
                Button::new("calendar-agenda")
                    .small()
                    .label("一覧")
                    .selected(self.calendar_mode == CalendarMode::Agenda)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.calendar_mode = CalendarMode::Agenda;
                            cx.notify();
                        });
                    })
            });

        let body = match self.calendar_mode {
            CalendarMode::Month => self.render_calendar_month(tasks, cx),
            CalendarMode::Agenda => self.render_calendar_agenda(tasks, cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h_0()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_calendar_agenda(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let mut dated = tasks
            .iter()
            .filter(|task| !matches!(task.due, Due::None))
            .cloned()
            .collect::<Vec<_>>();
        dated.sort_by_key(|task| format_due_input(&task.due));
        let undated = tasks
            .iter()
            .filter(|task| matches!(task.due, Due::None))
            .cloned()
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_1()
            .h_full()
            .gap_4()
            .p_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_2()
                    .overflow_y_scrollbar()
                    .child(section_label("納期順"))
                    .children(
                        dated
                            .into_iter()
                            .map(|task| self.render_calendar_item(task, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(300.0))
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(theme::SURFACE)
                    .child(section_label("納期未定"))
                    .children(
                        undated
                            .into_iter()
                            .map(|task| self.render_calendar_item(task, cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_calendar_month(&self, tasks: Vec<Task>, cx: &mut Context<Self>) -> AnyElement {
        let first = self.calendar_month;
        let now = OffsetDateTime::now_utc();
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let today = now.to_offset(offset).date();
        let undated = tasks
            .iter()
            .filter(|task| matches!(task.due, Due::None))
            .cloned()
            .collect::<Vec<_>>();
        let leading = calendar_leading_days(first);
        let days = first.month().length(first.year()) as usize;
        let mut cells = Vec::with_capacity(42);
        for cell in 0..42 {
            if cell < leading || cell >= leading + days {
                cells.push(
                    div()
                        .h(px(CALENDAR_DAY_CELL_HEIGHT))
                        .border_1()
                        .border_color(theme::BORDER)
                        .bg(theme::BACKGROUND)
                        .into_any_element(),
                );
                continue;
            }
            let day = (cell - leading + 1) as u8;
            let date = Date::from_calendar_date(first.year(), first.month(), day)
                .expect("calendar day must be valid");
            let day_tasks = tasks
                .iter()
                .filter(|task| due_date(&task.due) == Some(date))
                .take(3)
                .cloned()
                .collect::<Vec<_>>();
            let total = tasks
                .iter()
                .filter(|task| due_date(&task.due) == Some(date))
                .count();
            cells.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .h(px(CALENDAR_DAY_CELL_HEIGHT))
                    .p_2()
                    .border_1()
                    .border_color(if date == today {
                        theme::ACCENT
                    } else {
                        theme::BORDER
                    })
                    .bg(if date == today {
                        theme::SURFACE_HOVER
                    } else {
                        theme::SURFACE
                    })
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(if date == today {
                                theme::ACCENT
                            } else {
                                theme::TEXT
                            })
                            .child(if date == today {
                                format!("{day}  今日")
                            } else {
                                day.to_string()
                            }),
                    )
                    .children(day_tasks.into_iter().map(|task| {
                        let id = task.id;
                        div()
                            .id(SharedString::from(format!("month-{date}-{id}")))
                            .px_1()
                            .rounded_sm()
                            .bg(theme::BACKGROUND)
                            .border_l_2()
                            .border_color(priority_color(task.priority))
                            .text_size(px(12.0))
                            .text_color(if task.status == TaskStatus::Done {
                                theme::MUTED
                            } else {
                                theme::TEXT
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_task(id, window, cx);
                            }))
                            .child(task.title)
                    }))
                    .when(total > 3, |cell| {
                        cell.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme::MUTED)
                                .child(format!("ほか{}件", total - 3)),
                        )
                    })
                    .into_any_element(),
            );
        }
        let weekdays = ["日", "月", "火", "水", "木", "金", "土"];
        let calendar = div()
            .debug_selector(|| "calendar-month-grid".to_owned())
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_h(px(CALENDAR_GRID_MIN_HEIGHT))
            .child(
                div()
                    .grid()
                    .grid_cols(7)
                    .children(weekdays.into_iter().enumerate().map(|(index, day)| {
                        div()
                            .h(px(CALENDAR_WEEKDAY_HEIGHT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_center()
                            .text_color(match index {
                                0 => theme::DANGER,
                                6 => theme::ACCENT,
                                _ => theme::MUTED,
                            })
                            .child(day)
                    })),
            )
            .child(div().grid().grid_cols(7).children(cells));
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h_0()
            .gap_3()
            .p_4()
            .overflow_y_scrollbar()
            .child(calendar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap_2()
                    .overflow_x_scrollbar()
                    .child(section_label("納期未定"))
                    .when(undated.is_empty(), |panel| {
                        panel.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::MUTED)
                                .child("納期未定のタスクはありません"),
                        )
                    })
                    .children(undated.into_iter().map(|task| {
                        let id = task.id;
                        div()
                            .id(SharedString::from(format!("undated-{id}")))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::BORDER)
                            .bg(theme::SURFACE)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_task(id, window, cx);
                            }))
                            .child(task.title)
                    })),
            )
            .into_any_element()
    }

    fn render_calendar_item(&self, task: Task, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        div()
            .id(SharedString::from(format!("calendar-{id}")))
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme::BORDER)
            .bg(theme::SURFACE)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_task(id, window, cx);
            }))
            .child(
                div()
                    .w(px(150.0))
                    .text_color(theme::MUTED)
                    .child(format_due_display(&task.due)),
            )
            .child(task.title)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::TaskApplication,
        domain::ViewKind,
        infrastructure::{AppPaths, AppSettings, InstanceLock},
    };
    use gpui::{AppContext as _, Modifiers, VisualTestContext, size};
    use time::Month;

    #[gpui::test]
    fn month_navigation_keeps_buttons_anchored_and_returns_to_current_month(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::resolve(Some(directory.path())).unwrap();
        let lock = InstanceLock::acquire(&paths.lock).unwrap();
        let application = TaskApplication::start(&paths.database).unwrap();
        let snapshot = application.load().unwrap();
        cx.update(gpui_component::init);
        let mut workspace = None;
        let window = cx.add_window(|window, cx| {
            let entity = cx.new(|cx| {
                Workspace::new(
                    application,
                    snapshot,
                    paths,
                    AppSettings {
                        view_kind: ViewKind::Calendar,
                        ..AppSettings::default()
                    },
                    lock,
                    false,
                    window,
                    cx,
                )
            });
            workspace = Some(entity.clone());
            gpui_component::Root::new(entity, window, cx)
        });
        let workspace = workspace.unwrap();
        let mut visual = VisualTestContext::from_window(*window, cx);
        let september = Date::from_calendar_date(2026, Month::September, 1).unwrap();
        for width in [900.0, 1280.0] {
            visual.simulate_resize(size(px(width), px(800.0)));
            workspace.update_in(&mut visual, |this, _, cx| {
                this.calendar_month = september;
                cx.notify();
            });
            visual.run_until_parked();
            let selectors = [
                "calendar-previous",
                "calendar-month-label",
                "calendar-next",
                "calendar-current-month",
            ];
            let initial = selectors.map(|selector| visual.debug_bounds(selector).unwrap());
            // Icon-only buttons should be square, and all controls must stay visible.
            for button in [initial[0], initial[2]] {
                assert_eq!(button.size.width, button.size.height);
            }
            for bounds in initial {
                assert!(bounds.size.width > px(0.0));
                assert!(bounds.origin.x >= px(0.0) && bounds.right() <= px(width));
            }
            for pair in initial.windows(2) {
                assert!(pair[0].right() < pair[1].left());
            }

            // Cross both the one-/two-digit boundary and December/January in each direction.
            for (button, months) in [(initial[2], [1, 2, 3, 4]), (initial[0], [3, 2, 1, 0])] {
                for months in months {
                    visual.simulate_click(button.center(), Modifiers::default());
                    visual.run_until_parked();
                    workspace.update_in(&mut visual, |this, _, _| {
                        assert_eq!(this.calendar_month, shift_month(september, months));
                    });
                    for (selector, expected) in selectors.into_iter().zip(initial) {
                        assert_eq!(
                            visual.debug_bounds(selector).unwrap(),
                            expected,
                            "{selector} moved after shifting {months} months at width {width}",
                        );
                    }
                }
            }

            // Start far away so the current-month button cannot pass as a no-op.
            workspace.update_in(&mut visual, |this, _, cx| {
                this.calendar_month = Date::from_calendar_date(2000, Month::January, 1).unwrap();
                cx.notify();
            });
            visual.run_until_parked();
            visual.simulate_click(initial[3].center(), Modifiers::default());
            visual.run_until_parked();
            let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
            let today = OffsetDateTime::now_utc().to_offset(offset).date();
            workspace.update_in(&mut visual, |this, _, _| {
                assert_eq!(
                    this.calendar_month,
                    Date::from_calendar_date(today.year(), today.month(), 1).unwrap(),
                );
            });
            assert_eq!(visual.debug_bounds(selectors[3]).unwrap(), initial[3]);
        }
        visual.update(|window, _| window.remove_window());
    }
}
