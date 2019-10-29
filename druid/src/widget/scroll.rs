// Copyright 2019 The xi-editor Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A container that scrolls its contents.

use std::f64::INFINITY;
use std::time::{Duration, Instant};

use log::error;

use crate::{
    BaseState, BoxConstraints, Data, Env, Event, EventCtx, LayoutCtx, PaintCtx, Point, Rect, Size,
    TimerToken, UpdateCtx, Vec2, Widget, WidgetPod,
};

use crate::piet::RenderContext;
use crate::theme;

use crate::kurbo::{Affine, RoundedRect};

const SCROLL_BAR_WIDTH: f64 = 8.;
const SCROLL_BAR_PAD: f64 = 2.;

#[derive(Debug, Clone)]
enum ScrollDirection {
    Horizontal,
    Vertical,
    All,
}

impl ScrollDirection {
    /// Return the maximum size the container can be given
    /// its scroll direction and box constraints.
    /// In practice vertical scrolling will be width limited to
    /// box constraints and horizontal will be height limited.
    pub fn max_size(&self, bc: &BoxConstraints) -> Size {
        match self {
            ScrollDirection::Horizontal => Size::new(INFINITY, bc.max().height),
            ScrollDirection::Vertical => Size::new(bc.max().width, INFINITY),
            ScrollDirection::All => Size::new(INFINITY, INFINITY),
        }
    }
}

struct ScrollBarsState {
    opacity: f64,
    timer_id: TimerToken,
    hovered: bool,
    vertical_held: bool,
    horizontal_held: bool,
    held_offset: f64,
}

impl Default for ScrollBarsState {
    fn default() -> Self {
        Self {
            opacity: 0.0,
            timer_id: TimerToken::INVALID,
            hovered: false,
            vertical_held: false,
            horizontal_held: false,
            held_offset: 0f64,
        }
    }
}

/// A container that scrolls its contents.
///
/// This container holds a single child, and uses the wheel to scroll it
/// when the child's bounds are larger than the viewport.
///
/// The child is laid out with completely unconstrained layout bounds.
pub struct Scroll<T: Data, W: Widget<T>> {
    child: WidgetPod<T, W>,
    child_size: Size,
    scroll_offset: Vec2,
    direction: ScrollDirection,
    scroll_bars: ScrollBarsState,
}

impl<T: Data, W: Widget<T>> Scroll<T, W> {
    /// Create a new scroll container.
    ///
    /// This method will allow scrolling in all directions if child's bounds
    /// are larger than the viewport. Use [vertical](#method.vertical)
    /// and [horizontal](#method.horizontal) methods to limit scroll behavior.
    pub fn new(child: W) -> Scroll<T, W> {
        Scroll {
            child: WidgetPod::new(child),
            child_size: Default::default(),
            scroll_offset: Vec2::new(0.0, 0.0),
            direction: ScrollDirection::All,
            scroll_bars: ScrollBarsState::default(),
        }
    }

    /// Limit scroll behavior to allow only vertical scrolling (Y-axis).
    /// The child is laid out with constrained width and infinite height.
    pub fn vertical(mut self) -> Self {
        self.direction = ScrollDirection::Vertical;
        self
    }

    /// Limit scroll behavior to allow only horizontal scrolling (X-axis).
    /// The child is laid out with constrained height and infinite width.
    pub fn horizontal(mut self) -> Self {
        self.direction = ScrollDirection::Horizontal;
        self
    }

    /// Returns a reference to the child widget.
    pub fn child(&self) -> &W {
        self.child.widget()
    }

    /// Returns a mutable reference to the child widget.
    pub fn child_mut(&mut self) -> &mut W {
        self.child.widget_mut()
    }

    /// Update the scroll.
    ///
    /// Returns `true` if the scroll has been updated.
    pub fn scroll(&mut self, delta: Vec2, size: Size) -> bool {
        let mut offset = self.scroll_offset + delta;
        offset.x = offset.x.min(self.child_size.width - size.width).max(0.0);
        offset.y = offset.y.min(self.child_size.height - size.height).max(0.0);
        if (offset - self.scroll_offset).hypot2() > 1e-12 {
            self.scroll_offset = offset;
            true
        } else {
            false
        }
    }

    /// Makes the scrollbars visible, and resets the fade timer.
    pub fn reset_scrollbar_fade(&mut self, ctx: &mut EventCtx) {
        // Display scroll bars and schedule their disappearance
        self.scroll_bars.opacity = 0.7;
        let deadline = Instant::now() + Duration::from_millis(1500);
        self.scroll_bars.timer_id = ctx.request_timer(deadline);
    }

    /// Returns the current scroll offset.
    pub fn offset(&self) -> Vec2 {
        self.scroll_offset
    }

    fn calc_scollbar_bounds(&self, viewport: &Rect) -> Rect {
        return Rect::new(
            self.scroll_offset.x + SCROLL_BAR_PAD,
            self.scroll_offset.y + SCROLL_BAR_PAD,
            self.scroll_offset.x - SCROLL_BAR_PAD + viewport.width(),
            self.scroll_offset.y - SCROLL_BAR_PAD + viewport.height(),
        );
    }

    fn calc_vertical_bar_bounds(&self, viewport: &Rect) -> Rect {
        let scrollbar_bounds = self.calc_scollbar_bounds(viewport);

        let content_size = Size::new(
            viewport.width() - 2.0 * SCROLL_BAR_PAD,
            viewport.height() - 2.0 * SCROLL_BAR_PAD,
        );

        let scale_y = content_size.height / self.child_size.height;

        let h = (scale_y * content_size.height).ceil();
        let dh = (scale_y * self.scroll_offset.y).ceil();

        let x0 = scrollbar_bounds.x1 - SCROLL_BAR_WIDTH;
        let y0 = scrollbar_bounds.y0 + dh;

        let x1 = scrollbar_bounds.x1;
        let y1 = (y0 + h).min(scrollbar_bounds.y1);

        return Rect::new(x0, y0, x1, y1);
    }

    fn calc_horizontal_bar_bounds(&self, viewport: &Rect) -> Rect {
        let scrollbar_bounds = self.calc_scollbar_bounds(viewport);

        let content_size = Size::new(
            viewport.width() - 2.0 * SCROLL_BAR_PAD,
            viewport.height() - 2.0 * SCROLL_BAR_PAD,
        );

        let scale_x = content_size.width / self.child_size.width;

        let w = (scale_x * content_size.width).ceil();
        let dw = (scale_x * self.scroll_offset.x).ceil();

        let x0 = scrollbar_bounds.x0 + dw;
        let y0 = scrollbar_bounds.y1 - SCROLL_BAR_WIDTH;

        let x1 = (x0 + w).min(scrollbar_bounds.x1);
        let y1 = scrollbar_bounds.y1;

        return Rect::new(x0, y0, x1, y1);
    }

    /// Draw scroll bars.
    fn draw_bars(&self, paint_ctx: &mut PaintCtx, viewport: &Rect, env: &Env) {
        if self.scroll_bars.opacity <= 0.0 {
            return;
        }

        let brush = paint_ctx.render_ctx.solid_brush(
            env.get(theme::SCROLL_BAR_COLOR)
                .with_alpha(self.scroll_bars.opacity),
        );
        let border_brush = paint_ctx.render_ctx.solid_brush(
            env.get(theme::SCROLL_BAR_BORDER_COLOR)
                .with_alpha(self.scroll_bars.opacity),
        );

        // Vertical bar
        if viewport.height() < self.child_size.height {
            let rect = RoundedRect::from_rect(self.calc_vertical_bar_bounds(viewport), 5.0);
            paint_ctx.render_ctx.fill(rect, &brush);
            paint_ctx.render_ctx.stroke(rect, &border_brush, 1.0);
        }

        // Horizontal bar
        if viewport.width() < self.child_size.width {
            let rect = RoundedRect::from_rect(self.calc_horizontal_bar_bounds(viewport), 5.0);
            paint_ctx.render_ctx.fill(rect, &brush);
            paint_ctx.render_ctx.stroke(rect, &border_brush, 1.0);
        }
    }

    fn mouse_over_vertical_bar(&self, viewport: Rect, pos: Point) -> bool {
        if viewport.height() < self.child_size.height {
            return self.calc_vertical_bar_bounds(&viewport).contains(pos);
        }

        return false;
    }

    fn mouse_over_horizontal_bar(&self, viewport: Rect, pos: Point) -> bool {
        if viewport.width() < self.child_size.width {
            return self.calc_horizontal_bar_bounds(&viewport).contains(pos);
        }

        return false;
    }
}

impl<T: Data, W: Widget<T>> Widget<T> for Scroll<T, W> {
    fn paint(&mut self, paint_ctx: &mut PaintCtx, base_state: &BaseState, data: &T, env: &Env) {
        if let Err(e) = paint_ctx.save() {
            error!("saving render context failed: {:?}", e);
            return;
        }
        let viewport = Rect::from_origin_size(Point::ORIGIN, base_state.size());
        paint_ctx.clip(viewport);
        paint_ctx.transform(Affine::translate(-self.scroll_offset));

        let visible = viewport.with_origin(self.scroll_offset.to_point());
        paint_ctx.with_child_ctx(visible, |ctx| self.child.paint(ctx, data, env));

        self.draw_bars(paint_ctx, &viewport, env);

        if let Err(e) = paint_ctx.restore() {
            error!("restoring render context failed: {:?}", e);
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx, bc: &BoxConstraints, data: &T, env: &Env) -> Size {
        bc.debug_check("Scroll");

        let child_bc = BoxConstraints::new(Size::ZERO, self.direction.max_size(bc));
        let size = self.child.layout(ctx, &child_bc, data, env);
        self.child_size = size;
        self.child
            .set_layout_rect(Rect::from_origin_size(Point::ORIGIN, size));
        let self_size = bc.constrain(Size::new(100.0, 100.0));
        let _ = self.scroll(Vec2::new(0.0, 0.0), self_size);
        self_size
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx, data: &mut T, env: &Env) {
        let size = ctx.base_state.size();
        let viewport = Rect::from_origin_size(Point::ORIGIN, size);

        if match event {
            Event::MouseMoved(_) | Event::MouseUp(_) => {
                self.scroll_bars.vertical_held || self.scroll_bars.horizontal_held
            }
            _ => false,
        } {
            match event {
                Event::MouseMoved(event) => {
                    if self.scroll_bars.vertical_held {
                        let bounds = self.calc_vertical_bar_bounds(&viewport);
                        let mouse_y = event.pos.y + self.scroll_offset.y;
                        self.scroll(
                            Vec2::new(0f64, mouse_y - bounds.y0 - self.scroll_bars.held_offset),
                            size,
                        );
                    } else if self.scroll_bars.horizontal_held {
                        let bounds = self.calc_horizontal_bar_bounds(&viewport);
                        let mouse_x = event.pos.x + self.scroll_offset.x;
                        self.scroll(
                            Vec2::new(mouse_x - bounds.x0 - self.scroll_bars.held_offset, 0f64),
                            size,
                        );
                    }
                    ctx.invalidate();
                }
                Event::MouseUp(_) => {
                    self.scroll_bars.vertical_held = false;
                    self.scroll_bars.horizontal_held = false;
                }
                _ => (),
            }
        } else if match event {
            Event::MouseMoved(event) | Event::MouseDown(event) => {
                let mut transformed_event = event.clone();
                transformed_event.pos += self.scroll_offset;
                self.mouse_over_vertical_bar(viewport, transformed_event.pos)
                    || self.mouse_over_horizontal_bar(viewport, transformed_event.pos)
            }
            _ => false,
        } {
            match event {
                Event::MouseMoved(_) => {
                    self.scroll_bars.hovered = true;
                    self.scroll_bars.opacity = 0.7;
                    self.scroll_bars.timer_id = TimerToken::INVALID; // Cancel any fade out in progress
                    ctx.invalidate();
                }
                Event::MouseDown(event) => {
                    let pos = event.pos + self.scroll_offset;

                    if self.mouse_over_vertical_bar(viewport, pos) {
                        self.scroll_bars.vertical_held = true;
                        self.scroll_bars.held_offset =
                            pos.y - self.calc_vertical_bar_bounds(&viewport).y0;
                    } else if self.mouse_over_horizontal_bar(viewport, pos) {
                        self.scroll_bars.horizontal_held = true;
                        self.scroll_bars.held_offset =
                            pos.x - self.calc_horizontal_bar_bounds(&viewport).x0;
                    }
                }
                _ => (),
            }
        } else {
            let child_event = event.transform_scroll(self.scroll_offset, viewport);
            if let Some(child_event) = child_event {
                self.child.event(&child_event, ctx, data, env)
            };

            match event {
                Event::MouseMoved(event) => {
                    let mut transformed_event = event.clone();
                    transformed_event.pos += self.scroll_offset;
                    let currently_hovered = self
                        .mouse_over_vertical_bar(viewport, transformed_event.pos)
                        || self.mouse_over_horizontal_bar(viewport, transformed_event.pos);
                    if self.scroll_bars.hovered && !currently_hovered {
                        self.scroll_bars.hovered = false;
                        self.reset_scrollbar_fade(ctx);
                    }
                }
                // Show the scrollbars any time our size changes
                Event::Size(_) => self.reset_scrollbar_fade(ctx),
                // The scroll bars will fade immediately if there's some other widget requesting animation.
                // Guard by the timer id being invalid.
                Event::AnimFrame(interval) if self.scroll_bars.timer_id == TimerToken::INVALID => {
                    // Animate scroll bars opacity
                    let diff = 2.0 * (*interval as f64) * 1e-9;
                    self.scroll_bars.opacity -= diff;
                    if self.scroll_bars.opacity > 0.0 {
                        ctx.request_anim_frame();
                    }
                }
                Event::Timer(id) if *id == self.scroll_bars.timer_id => {
                    // Schedule scroll bars animation
                    ctx.request_anim_frame();
                    self.scroll_bars.timer_id = TimerToken::INVALID;
                }
                _ => (),
            }
        }

        if !ctx.is_handled() {
            if let Event::Wheel(wheel) = event {
                if self.scroll(wheel.delta, size) {
                    ctx.invalidate();
                    ctx.set_handled();
                    self.reset_scrollbar_fade(ctx);
                }
            }
        }
    }

    fn update(&mut self, ctx: &mut UpdateCtx, _old_data: Option<&T>, data: &T, env: &Env) {
        self.child.update(ctx, data, env);
    }
}
