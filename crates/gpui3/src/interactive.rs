use parking_lot::Mutex;
use smallvec::SmallVec;

use crate::{
    point, Action, Bounds, DispatchContext, DispatchPhase, Element, FocusHandle, Keystroke,
    Modifiers, Pixels, Point, ViewContext,
};
use std::{
    any::{Any, TypeId},
    mem,
    ops::Deref,
    sync::Arc,
};

pub trait Interactive: Element {
    fn interactivity(&mut self) -> &mut Interactivity<Self::ViewState>;

    fn on_mouse_down(
        mut self,
        button: MouseButton,
        handler: impl Fn(&mut Self::ViewState, &MouseDownEvent, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .mouse_down
            .push(Arc::new(move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == button
                    && bounds.contains_point(&event.position)
                {
                    handler(view, event, cx)
                }
            }));
        self
    }

    fn on_mouse_up(
        mut self,
        button: MouseButton,
        handler: impl Fn(&mut Self::ViewState, &MouseUpEvent, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .mouse_up
            .push(Arc::new(move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == button
                    && bounds.contains_point(&event.position)
                {
                    handler(view, event, cx)
                }
            }));
        self
    }

    fn on_mouse_down_out(
        mut self,
        button: MouseButton,
        handler: impl Fn(&mut Self::ViewState, &MouseDownEvent, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .mouse_down
            .push(Arc::new(move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == button
                    && !bounds.contains_point(&event.position)
                {
                    handler(view, event, cx)
                }
            }));
        self
    }

    fn on_mouse_up_out(
        mut self,
        button: MouseButton,
        handler: impl Fn(&mut Self::ViewState, &MouseUpEvent, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .mouse_up
            .push(Arc::new(move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == button
                    && !bounds.contains_point(&event.position)
                {
                    handler(view, event, cx);
                }
            }));
        self
    }

    fn on_mouse_move(
        mut self,
        handler: impl Fn(&mut Self::ViewState, &MouseMoveEvent, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .mouse_move
            .push(Arc::new(move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    handler(view, event, cx);
                }
            }));
        self
    }

    fn on_scroll_wheel(
        mut self,
        handler: impl Fn(&mut Self::ViewState, &ScrollWheelEvent, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .scroll_wheel
            .push(Arc::new(move |view, event, bounds, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    handler(view, event, cx);
                }
            }));
        self
    }

    fn on_key_down(
        mut self,
        listener: impl Fn(
                &mut Self::ViewState,
                &KeyDownEvent,
                DispatchPhase,
                &mut ViewContext<Self::ViewState>,
            ) + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().key.push((
            TypeId::of::<KeyDownEvent>(),
            Arc::new(move |view, event, _, phase, cx| {
                let event = event.downcast_ref().unwrap();
                listener(view, event, phase, cx);
                None
            }),
        ));
        self
    }

    fn on_key_up(
        mut self,
        listener: impl Fn(&mut Self::ViewState, &KeyUpEvent, DispatchPhase, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().key.push((
            TypeId::of::<KeyUpEvent>(),
            Arc::new(move |view, event, _, phase, cx| {
                let event = event.downcast_ref().unwrap();
                listener(view, event, phase, cx);
                None
            }),
        ));
        self
    }

    fn on_action<A: 'static>(
        mut self,
        listener: impl Fn(&mut Self::ViewState, &A, DispatchPhase, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().key.push((
            TypeId::of::<A>(),
            Arc::new(move |view, event, _, phase, cx| {
                let event = event.downcast_ref().unwrap();
                listener(view, event, phase, cx);
                None
            }),
        ));
        self
    }
}

pub trait Click: Interactive {
    fn on_click(
        mut self,
        handler: impl Fn(&mut Self::ViewState, &MouseClickEvent, &mut ViewContext<Self::ViewState>)
            + Send
            + Sync
            + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity()
            .mouse_click
            .push(Arc::new(move |view, event, cx| handler(view, event, cx)));
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyDownEvent {
    pub keystroke: Keystroke,
    pub is_held: bool,
}

#[derive(Clone, Debug)]
pub struct KeyUpEvent {
    pub keystroke: Keystroke,
}

#[derive(Clone, Debug, Default)]
pub struct ModifiersChangedEvent {
    pub modifiers: Modifiers,
}

impl Deref for ModifiersChangedEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

/// The phase of a touch motion event.
/// Based on the winit enum of the same name.
#[derive(Clone, Copy, Debug)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
}

#[derive(Clone, Debug, Default)]
pub struct MouseDownEvent {
    pub button: MouseButton,
    pub position: Point<Pixels>,
    pub modifiers: Modifiers,
    pub click_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct MouseUpEvent {
    pub button: MouseButton,
    pub position: Point<Pixels>,
    pub modifiers: Modifiers,
    pub click_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct MouseClickEvent {
    pub down: MouseDownEvent,
    pub up: MouseUpEvent,
}

#[derive(Hash, PartialEq, Eq, Copy, Clone, Debug)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Navigate(NavigationDirection),
}

impl MouseButton {
    pub fn all() -> Vec<Self> {
        vec![
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Navigate(NavigationDirection::Back),
            MouseButton::Navigate(NavigationDirection::Forward),
        ]
    }
}

impl Default for MouseButton {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Hash, PartialEq, Eq, Copy, Clone, Debug)]
pub enum NavigationDirection {
    Back,
    Forward,
}

impl Default for NavigationDirection {
    fn default() -> Self {
        Self::Back
    }
}

#[derive(Clone, Debug, Default)]
pub struct MouseMoveEvent {
    pub position: Point<Pixels>,
    pub pressed_button: Option<MouseButton>,
    pub modifiers: Modifiers,
}

#[derive(Clone, Debug)]
pub struct ScrollWheelEvent {
    pub position: Point<Pixels>,
    pub delta: ScrollDelta,
    pub modifiers: Modifiers,
    pub touch_phase: TouchPhase,
}

impl Deref for ScrollWheelEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ScrollDelta {
    Pixels(Point<Pixels>),
    Lines(Point<f32>),
}

impl Default for ScrollDelta {
    fn default() -> Self {
        Self::Lines(Default::default())
    }
}

impl ScrollDelta {
    pub fn precise(&self) -> bool {
        match self {
            ScrollDelta::Pixels(_) => true,
            ScrollDelta::Lines(_) => false,
        }
    }

    pub fn pixel_delta(&self, line_height: Pixels) -> Point<Pixels> {
        match self {
            ScrollDelta::Pixels(delta) => *delta,
            ScrollDelta::Lines(delta) => point(line_height * delta.x, line_height * delta.y),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MouseExitEvent {
    pub position: Point<Pixels>,
    pub pressed_button: Option<MouseButton>,
    pub modifiers: Modifiers,
}

impl Deref for MouseExitEvent {
    type Target = Modifiers;

    fn deref(&self) -> &Self::Target {
        &self.modifiers
    }
}

#[derive(Clone, Debug)]
pub enum InputEvent {
    KeyDown(KeyDownEvent),
    KeyUp(KeyUpEvent),
    ModifiersChanged(ModifiersChangedEvent),
    MouseDown(MouseDownEvent),
    MouseUp(MouseUpEvent),
    MouseMoved(MouseMoveEvent),
    MouseExited(MouseExitEvent),
    ScrollWheel(ScrollWheelEvent),
}

impl InputEvent {
    pub fn position(&self) -> Option<Point<Pixels>> {
        match self {
            InputEvent::KeyDown { .. } => None,
            InputEvent::KeyUp { .. } => None,
            InputEvent::ModifiersChanged { .. } => None,
            InputEvent::MouseDown(event) => Some(event.position),
            InputEvent::MouseUp(event) => Some(event.position),
            InputEvent::MouseMoved(event) => Some(event.position),
            InputEvent::MouseExited(event) => Some(event.position),
            InputEvent::ScrollWheel(event) => Some(event.position),
        }
    }

    pub fn mouse_event<'a>(&'a self) -> Option<&'a dyn Any> {
        match self {
            InputEvent::KeyDown { .. } => None,
            InputEvent::KeyUp { .. } => None,
            InputEvent::ModifiersChanged { .. } => None,
            InputEvent::MouseDown(event) => Some(event),
            InputEvent::MouseUp(event) => Some(event),
            InputEvent::MouseMoved(event) => Some(event),
            InputEvent::MouseExited(event) => Some(event),
            InputEvent::ScrollWheel(event) => Some(event),
        }
    }

    pub fn keyboard_event<'a>(&'a self) -> Option<&'a dyn Any> {
        match self {
            InputEvent::KeyDown(event) => Some(event),
            InputEvent::KeyUp(event) => Some(event),
            InputEvent::ModifiersChanged(event) => Some(event),
            InputEvent::MouseDown(_) => None,
            InputEvent::MouseUp(_) => None,
            InputEvent::MouseMoved(_) => None,
            InputEvent::MouseExited(_) => None,
            InputEvent::ScrollWheel(_) => None,
        }
    }
}

pub struct FocusEvent {
    pub blurred: Option<FocusHandle>,
    pub focused: Option<FocusHandle>,
}

pub type MouseDownListener<V> = Arc<
    dyn Fn(&mut V, &MouseDownEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>)
        + Send
        + Sync
        + 'static,
>;
pub type MouseUpListener<V> = Arc<
    dyn Fn(&mut V, &MouseUpEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>)
        + Send
        + Sync
        + 'static,
>;
pub type MouseClickListener<V> =
    Arc<dyn Fn(&mut V, &MouseClickEvent, &mut ViewContext<V>) + Send + Sync + 'static>;

pub type MouseMoveListener<V> = Arc<
    dyn Fn(&mut V, &MouseMoveEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>)
        + Send
        + Sync
        + 'static,
>;

pub type ScrollWheelListener<V> = Arc<
    dyn Fn(&mut V, &ScrollWheelEvent, &Bounds<Pixels>, DispatchPhase, &mut ViewContext<V>)
        + Send
        + Sync
        + 'static,
>;

pub type KeyListener<V> = Arc<
    dyn Fn(
            &mut V,
            &dyn Any,
            &[&DispatchContext],
            DispatchPhase,
            &mut ViewContext<V>,
        ) -> Option<Box<dyn Action>>
        + Send
        + Sync
        + 'static,
>;

pub struct Interactivity<V> {
    pub mouse_down: SmallVec<[MouseDownListener<V>; 2]>,
    pub mouse_up: SmallVec<[MouseUpListener<V>; 2]>,
    pub mouse_click: SmallVec<[MouseClickListener<V>; 2]>,
    pub mouse_move: SmallVec<[MouseMoveListener<V>; 2]>,
    pub scroll_wheel: SmallVec<[ScrollWheelListener<V>; 2]>,
    pub key: SmallVec<[(TypeId, KeyListener<V>); 32]>,
}

impl<V> Default for Interactivity<V> {
    fn default() -> Self {
        Self {
            mouse_down: SmallVec::new(),
            mouse_up: SmallVec::new(),
            mouse_click: SmallVec::new(),
            mouse_move: SmallVec::new(),
            scroll_wheel: SmallVec::new(),
            key: SmallVec::new(),
        }
    }
}

impl<V> Interactivity<V>
where
    V: 'static + Send + Sync,
{
    pub fn paint(
        &mut self,
        bounds: Bounds<Pixels>,
        pending_click: Arc<Mutex<Option<MouseDownEvent>>>,
        cx: &mut ViewContext<V>,
    ) {
        let click_listeners = mem::take(&mut self.mouse_click);

        let mouse_down = pending_click.lock().clone();
        if let Some(mouse_down) = mouse_down {
            cx.on_mouse_event(move |state, event: &MouseUpEvent, phase, cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    let mouse_click = MouseClickEvent {
                        down: mouse_down.clone(),
                        up: event.clone(),
                    };
                    for listener in &click_listeners {
                        listener(state, &mouse_click, cx);
                    }
                }

                *pending_click.lock() = None;
            });
        } else {
            cx.on_mouse_event(move |_state, event: &MouseDownEvent, phase, _cx| {
                if phase == DispatchPhase::Bubble && bounds.contains_point(&event.position) {
                    *pending_click.lock() = Some(event.clone());
                }
            });
        }

        for listener in mem::take(&mut self.mouse_down) {
            cx.on_mouse_event(move |state, event: &MouseDownEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }

        for listener in mem::take(&mut self.mouse_up) {
            cx.on_mouse_event(move |state, event: &MouseUpEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }

        for listener in mem::take(&mut self.mouse_move) {
            cx.on_mouse_event(move |state, event: &MouseMoveEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }

        for listener in mem::take(&mut self.scroll_wheel) {
            cx.on_mouse_event(move |state, event: &ScrollWheelEvent, phase, cx| {
                listener(state, event, &bounds, phase, cx);
            })
        }
    }
}
