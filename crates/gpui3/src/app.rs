mod async_context;
mod model_context;

pub use async_context::*;
pub use model_context::*;

use crate::{
    current_platform, Context, LayoutId, MainThreadOnly, Platform, RootView, TextSystem, Window,
    WindowContext, WindowHandle, WindowId,
};
use anyhow::{anyhow, Result};
use collections::{HashMap, VecDeque};
use futures::{future, Future};
use parking_lot::Mutex;
use slotmap::SlotMap;
use smallvec::SmallVec;
use std::{
    any::Any,
    marker::PhantomData,
    sync::{Arc, Weak},
};
use util::ResultExt;

#[derive(Clone)]
pub struct App(Arc<Mutex<AppContext>>);

impl App {
    pub fn production() -> Self {
        Self::new(current_platform())
    }

    #[cfg(any(test, feature = "test"))]
    pub fn test() -> Self {
        Self::new(Arc::new(super::TestPlatform::new()))
    }

    fn new(platform: Arc<dyn Platform>) -> Self {
        let dispatcher = platform.dispatcher();
        let text_system = Arc::new(TextSystem::new(platform.text_system()));
        let mut entities = SlotMap::with_key();
        let unit_entity = Handle::new(entities.insert(Some(Box::new(()) as Box<dyn Any + Send>)));
        Self(Arc::new_cyclic(|this| {
            Mutex::new(AppContext {
                this: this.clone(),
                platform: MainThreadOnly::new(platform, dispatcher),
                text_system,
                unit_entity,
                entities,
                windows: SlotMap::with_key(),
                pending_updates: 0,
                pending_effects: Default::default(),
                observers: Default::default(),
                layout_id_buffer: Default::default(),
            })
        }))
    }

    pub fn run<F>(self, on_finish_launching: F)
    where
        F: 'static + FnOnce(&mut AppContext),
    {
        let this = self.clone();
        let platform = self.0.lock().platform.clone();
        platform.borrow_on_main_thread().run(Box::new(move || {
            let cx = &mut *this.0.lock();
            on_finish_launching(cx);
        }));
    }
}

type Handlers = SmallVec<[Arc<dyn Fn(&mut AppContext) -> bool + Send + Sync + 'static>; 2]>;

pub struct AppContext {
    this: Weak<Mutex<AppContext>>,
    platform: MainThreadOnly<dyn Platform>,
    text_system: Arc<TextSystem>,
    pending_updates: usize,
    pub(crate) unit_entity: Handle<()>,
    pub(crate) entities: SlotMap<EntityId, Option<Box<dyn Any + Send>>>,
    pub(crate) windows: SlotMap<WindowId, Option<Window>>,
    pub(crate) pending_effects: VecDeque<Effect>,
    pub(crate) observers: HashMap<EntityId, Handlers>,
    pub(crate) layout_id_buffer: Vec<LayoutId>, // We recycle this memory across layout requests.
}

impl AppContext {
    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    pub fn to_async(&self) -> AsyncContext {
        AsyncContext(self.this.clone())
    }

    pub fn spawn_on_main<F, R>(
        &self,
        f: impl FnOnce(&dyn Platform, &mut Self) -> F + Send + 'static,
    ) -> impl Future<Output = R>
    where
        F: Future<Output = R> + 'static,
        R: Send + 'static,
    {
        let this = self.this.upgrade().unwrap();
        self.platform.read(move |platform| {
            let cx = &mut *this.lock();
            cx.update(|cx| f(platform, cx))
        })
    }

    pub fn open_window<S: 'static + Send + Sync>(
        &mut self,
        options: crate::WindowOptions,
        build_root_view: impl FnOnce(&mut WindowContext) -> RootView<S> + Send + 'static,
    ) -> impl Future<Output = WindowHandle<S>> {
        let id = self.windows.insert(None);
        let handle = WindowHandle::new(id);
        self.spawn_on_main(move |platform, cx| {
            let mut window = Window::new(handle.into(), options, platform, cx);
            let root_view = build_root_view(&mut WindowContext::mutable(cx, &mut window));
            window.root_view.replace(root_view.into_any());
            cx.windows.get_mut(id).unwrap().replace(window);
            future::ready(handle)
        })
    }

    pub(crate) fn update_window<R>(
        &mut self,
        id: WindowId,
        update: impl FnOnce(&mut WindowContext) -> R,
    ) -> Result<R> {
        self.update(|cx| {
            let mut window = cx
                .windows
                .get_mut(id)
                .ok_or_else(|| anyhow!("window not found"))?
                .take()
                .unwrap();

            let result = update(&mut WindowContext::mutable(cx, &mut window));
            window.dirty = true;

            cx.windows
                .get_mut(id)
                .ok_or_else(|| anyhow!("window not found"))?
                .replace(window);

            Ok(result)
        })
    }

    fn update<R>(&mut self, update: impl FnOnce(&mut Self) -> R) -> R {
        self.pending_updates += 1;
        let result = update(self);
        self.pending_updates -= 1;
        if self.pending_updates == 0 {
            self.flush_effects();
        }
        result
    }

    fn flush_effects(&mut self) {
        while let Some(effect) = self.pending_effects.pop_front() {
            match effect {
                Effect::Notify(entity_id) => self.apply_notify_effect(entity_id),
            }
        }

        let dirty_window_ids = self
            .windows
            .iter()
            .filter_map(|(window_id, window)| {
                let window = window.as_ref().unwrap();
                if window.dirty {
                    Some(window_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for dirty_window_id in dirty_window_ids {
            self.update_window(dirty_window_id, |cx| cx.draw())
                .unwrap() // We know we have the window.
                .log_err();
        }
    }

    fn apply_notify_effect(&mut self, updated_entity: EntityId) {
        if let Some(mut handlers) = self.observers.remove(&updated_entity) {
            handlers.retain(|handler| handler(self));
            if let Some(new_handlers) = self.observers.remove(&updated_entity) {
                handlers.extend(new_handlers);
            }
            self.observers.insert(updated_entity, handlers);
        }
    }
}

impl Context for AppContext {
    type EntityContext<'a, 'w, T: Send + Sync + 'static> = ModelContext<'a, T>;
    type Result<T> = T;

    fn entity<T: Send + Sync + 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Self::EntityContext<'_, '_, T>) -> T,
    ) -> Handle<T> {
        let id = self.entities.insert(None);
        let entity = Box::new(build_entity(&mut ModelContext::mutable(self, id)));
        self.entities.get_mut(id).unwrap().replace(entity);

        Handle::new(id)
    }

    fn update_entity<T: Send + Sync + 'static, R>(
        &mut self,
        handle: &Handle<T>,
        update: impl FnOnce(&mut T, &mut Self::EntityContext<'_, '_, T>) -> R,
    ) -> R {
        let mut entity = self
            .entities
            .get_mut(handle.id)
            .unwrap()
            .take()
            .unwrap()
            .downcast::<T>()
            .unwrap();

        let result = update(&mut *entity, &mut ModelContext::mutable(self, handle.id));
        self.entities.get_mut(handle.id).unwrap().replace(entity);
        result
    }
}

slotmap::new_key_type! { pub struct EntityId; }

pub struct Handle<T> {
    pub(crate) id: EntityId,
    pub(crate) entity_type: PhantomData<T>,
}

impl<T: Send + Sync + 'static> Handle<T> {
    fn new(id: EntityId) -> Self {
        Self {
            id,
            entity_type: PhantomData,
        }
    }

    pub fn downgrade(&self) -> WeakHandle<T> {
        WeakHandle {
            id: self.id,
            entity_type: self.entity_type,
        }
    }

    /// Update the entity referenced by this handle with the given function.
    ///
    /// The update function receives a context appropriate for its environment.
    /// When updating in an `AppContext`, it receives a `ModelContext`.
    /// When updating an a `WindowContext`, it receives a `ViewContext`.
    pub fn update<C: Context, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut T, &mut C::EntityContext<'_, '_, T>) -> R,
    ) -> C::Result<R> {
        cx.update_entity(self, update)
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            entity_type: PhantomData,
        }
    }
}

pub struct WeakHandle<T> {
    pub(crate) id: EntityId,
    pub(crate) entity_type: PhantomData<T>,
}

impl<T: Send + Sync + 'static> WeakHandle<T> {
    pub fn upgrade(&self, _: &impl Context) -> Option<Handle<T>> {
        // todo!("Actually upgrade")
        Some(Handle {
            id: self.id,
            entity_type: self.entity_type,
        })
    }

    /// Update the entity referenced by this handle with the given function if
    /// the referenced entity still exists. Returns an error if the entity has
    /// been released.
    ///
    /// The update function receives a context appropriate for its environment.
    /// When updating in an `AppContext`, it receives a `ModelContext`.
    /// When updating an a `WindowContext`, it receives a `ViewContext`.
    pub fn update<C: Context, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut T, &mut C::EntityContext<'_, '_, T>) -> R,
    ) -> Result<R>
    where
        Result<C::Result<R>>: crate::Flatten<R>,
    {
        crate::Flatten::flatten(
            self.upgrade(cx)
                .ok_or_else(|| anyhow!("entity release"))
                .map(|this| cx.update_entity(&this, update)),
        )
    }
}

pub(crate) enum Effect {
    Notify(EntityId),
}

#[cfg(test)]
mod tests {
    use super::AppContext;

    #[test]
    fn test_app_context_send_sync() {
        // This will not compile if `AppContext` does not implement `Send`
        fn assert_send<T: Send>() {}
        assert_send::<AppContext>();
    }
}
