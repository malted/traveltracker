// Taken from  the amazing `system_status_bar_macos` crate by amachang!
// Star it at https://github.com/amachang/system_status_bar_macos

use std::{
    ffi::c_void,
    future::Future,
    ptr::NonNull,
    sync::mpsc::{channel, Receiver, Sender, TryRecvError},
    thread::sleep,
    time::Duration,
};

use objc2::{
    declare::{Ivar, IvarDrop},
    declare_class,
    ffi::{objc_autoreleasePoolPop, objc_autoreleasePoolPush},
    msg_send, msg_send_id,
    mutability::InteriorMutable,
    rc::Id,
    runtime::NSObject,
    sel, ClassType,
};

use icrate::{
    AppKit::{
        NSAlert, NSApplication, NSEvent, NSEventMaskAny, NSImage, NSMenu, NSMenuItem, NSStatusBar,
        NSStatusItem, NSVariableStatusItemLength,
    },
    Foundation::NSString,
};

use block2::{Block, ConcreteBlock, RcBlock};

#[derive(Debug)]
pub struct StatusItem {
    inner: Id<NSStatusItem>,

    menu: Menu,
    title: String,
}

impl StatusItem {
    pub fn new(title: impl AsRef<str>, menu: Menu) -> Self {
        // not testable function (it bounds to the main thread)
        unsafe {
            // initialize if not yet
            NSApplication::sharedApplication();

            let bar = NSStatusBar::systemStatusBar();
            let inner = bar.statusItemWithLength(NSVariableStatusItemLength);

            Self::new_impl(inner, title, menu)
        }
    }

    fn new_impl(inner: Id<NSStatusItem>, title: impl AsRef<str>, menu: Menu) -> Self {
        let title = title.as_ref();
        // testable part of new function
        unsafe {
            inner.setMenu(Some(&menu.inner));
            inner
                .button()
                .map(|b| b.setTitle(&NSString::from_str(title)));

            let title = title.to_string();
            Self { inner, menu, title }
        }
    }

    pub fn menu(&self) -> &Menu {
        &self.menu
    }

    pub fn set_menu(&mut self, menu: Menu) {
        unsafe {
            self.inner.setMenu(Some(&menu.inner));
            self.menu = menu;
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: impl AsRef<str>) {
        let title = title.as_ref();
        unsafe {
            self.inner
                .button()
                .map(|b| b.setTitle(&NSString::from_str(title)));
            self.title = title.to_string();
        }
    }

    pub fn set_image(&mut self, system_image_name: impl AsRef<str>) {
        let system_image_name = system_image_name.as_ref();
        unsafe {
            self.inner.button().map(|b| {
                let img = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &*NSString::from_str(system_image_name),
                    Some(&*NSString::from_str(&format!("{system_image_name} icon"))),
                )
                .expect("an NSImage from the given system symbol name");

                b.setImage(Some(&*img));
            });
        }
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        unsafe {
            self.inner.setMenu(None);

            let bar = NSStatusBar::systemStatusBar();
            bar.removeStatusItem(&self.inner);
        }
    }
}

#[derive(Debug)]
pub struct Menu {
    inner: Id<NSMenu>,

    items: Vec<MenuItem>,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        unsafe {
            let inner = NSMenu::new();

            for item in &items {
                inner.addItem(&item.inner);
            }

            Self { inner, items }
        }
    }

    pub fn items(&self) -> &Vec<MenuItem> {
        &self.items
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            self.inner.removeAllItems();
            self.items = vec![];
        }
    }
}

declare_class!(
    #[derive(Debug)]
    struct STBMenuItemCallback {
        callback: IvarDrop<Box<RcBlock<(*mut NSMenuItem,), ()>>, "_callback">,
    }

    mod ivars;

    unsafe impl ClassType for STBMenuItemCallback {
        type Super = NSObject;
        type Mutability = InteriorMutable;
        const NAME: &'static str = "STBMenuItemCallback";
    }

    unsafe impl STBMenuItemCallback {
        #[method(initWithCallback:)]
        unsafe fn init(
            this: *mut Self,
            callback: *mut Block<(*mut NSMenuItem,), ()>,
        ) -> Option<NonNull<Self>> {
            let this: Option<&mut Self> = msg_send![super(this), init];
            let Some(this) = this else {
                return None;
            };

            Ivar::write(&mut this.callback, Box::new(RcBlock::copy(callback)));

            Some(NonNull::from(this))
        }

        #[method(call:)]
        unsafe fn call(&self, sender: *mut NSMenuItem) {
            self.callback.call((sender,));
        }
    }
);

impl STBMenuItemCallback {
    fn new(callback: &Block<(*mut NSMenuItem,), ()>) -> Id<Self> {
        unsafe { msg_send_id![Self::alloc(), initWithCallback: callback] }
    }
}

#[derive(Debug)]
pub struct MenuItem {
    inner: Id<NSMenuItem>,

    title: String,
    callback: Option<MenuItemCallback>,
    submenu: Option<Menu>,
}

impl MenuItem {
    pub fn new(
        title: impl AsRef<str>,
        callback: Option<Box<dyn Fn() + 'static>>,
        submenu: Option<Menu>,
    ) -> Self {
        let title = title.as_ref();
        unsafe {
            let inner = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            );

            let callback = callback.map(|callback| {
                let callback = MenuItemCallback::new(callback);
                inner.setTarget(Some(&callback.inner));
                inner.setAction(Some(sel!(call:)));
                callback
            });

            let submenu = submenu.map(|submenu| {
                inner.setSubmenu(Some(&submenu.inner));
                submenu
            });

            let title = title.to_string();
            Self {
                inner,
                title,
                callback,
                submenu,
            }
        }
    }

    pub fn submenu(&self) -> Option<&Menu> {
        self.submenu.as_ref()
    }

    pub fn title(&self) -> &str {
        &self.title
    }
}

impl Drop for MenuItem {
    fn drop(&mut self) {
        unsafe {
            self.inner.setTarget(None);
            self.inner.setAction(None);
            self.callback = None;
            self.inner.setSubmenu(None);
            self.submenu = None;
        }
    }
}

#[derive(Debug)]
struct MenuItemCallback {
    inner: Id<STBMenuItemCallback>,
}

impl MenuItemCallback {
    fn new(callback: Box<dyn Fn() + 'static>) -> Self {
        let callback_block = ConcreteBlock::new(move |_: *mut NSMenuItem| {
            callback();
        })
        .copy();
        let inner = STBMenuItemCallback::new(&*callback_block);
        Self { inner }
    }
}

#[derive(Debug)]
pub struct LoopTerminator {
    sender: Sender<()>,
}

impl LoopTerminator {
    fn new() -> (Self, LoopTerminatee) {
        let (sender, receiver) = channel::<()>();
        (Self { sender }, LoopTerminatee { receiver })
    }

    pub fn terminate(&self) {
        self.sender.send(()).unwrap();
    }
}

#[derive(Debug)]
struct LoopTerminatee {
    receiver: Receiver<()>,
}

impl LoopTerminatee {
    fn should_terminate(&self) -> bool {
        match self.receiver.try_recv() {
            Ok(()) => true,
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => true,
        }
    }
}

#[derive(Debug)]
struct NopLoopTerminatee {}

impl NopLoopTerminatee {
    fn should_terminate(&self) -> bool {
        false
    }
}

struct AutoReleasePoolContext(*mut c_void);
unsafe impl Send for AutoReleasePoolContext {}

macro_rules! event_loop {
    ($terminatee: expr, $sleep: expr, $receiver_callback: expr) => {
        unsafe {
            let run_mode = NSString::from_str("kCFRunLoopDefaultMode");
            {
                let app = NSApplication::sharedApplication();
                app.finishLaunching();
            }
            'event_loop: loop {
                let pool_ctx = AutoReleasePoolContext(objc_autoreleasePoolPush());
                for _ in 0..100 {
                    {
                        let app = NSApplication::sharedApplication();
                        if $terminatee.should_terminate() {
                            break 'event_loop;
                        }

                        $receiver_callback;

                        let event: Option<Id<NSEvent>> = app
                            .nextEventMatchingMask_untilDate_inMode_dequeue(
                                NSEventMaskAny,
                                None,
                                &run_mode,
                                true,
                            );
                        if let Some(event) = event {
                            app.sendEvent(&event);
                        };
                        app.updateWindows();
                    }
                    $sleep;
                }
                objc_autoreleasePoolPop(pool_ctx.0);
            }
        };
    };
}

pub fn sync_event_loop<T>(
    receiver: Receiver<T>,
    callback: impl Fn(T),
) -> (impl Fn(), LoopTerminator) {
    let (terminator, terminatee) = LoopTerminator::new();
    let f = move || {
        event_loop!(
            terminatee,
            sleep(Duration::from_millis(10)),
            if let Ok(data) = receiver.try_recv() {
                callback(data)
            }
        );
    };
    (f, terminator)
}

pub fn sync_infinite_event_loop<T>(receiver: Receiver<T>, callback: impl Fn(T)) {
    let terminatee = NopLoopTerminatee {};
    event_loop!(
        terminatee,
        sleep(Duration::from_millis(10)),
        if let Ok(data) = receiver.try_recv() {
            callback(data)
        }
    );
}

pub fn async_event_loop<F>(
    async_sleep: impl Fn(Duration) -> F,
) -> (impl Future<Output = ()>, LoopTerminator)
where
    F: Future<Output = ()>,
{
    let (terminator, terminatee) = LoopTerminator::new();
    let future = async move {
        event_loop!(terminatee, async_sleep(Duration::from_millis(10)).await, ());
    };
    (future, terminator)
}

pub fn async_infinite_event_loop<F>(async_sleep: impl Fn(Duration) -> F) -> impl Future<Output = ()>
where
    F: Future<Output = ()>,
{
    let terminatee = NopLoopTerminatee {};
    let future = async move {
        event_loop!(terminatee, async_sleep(Duration::from_millis(10)).await, ());
    };
    future
}

#[cfg(test)]
mod tests {
    use crate::*;
    use icrate::Foundation::*;
    use std::{cell::*, rc::*, thread::*};

    #[test]
    fn construct_menu() {
        unsafe {
            let status_item = StatusItem::new_impl(
                NSStatusItem::new(),
                "000",
                Menu::new(vec![
                    MenuItem::new("001", None, None),
                    MenuItem::new(
                        "002",
                        None,
                        Some(Menu::new(vec![
                            MenuItem::new("003", None, None),
                            MenuItem::new("004", None, None),
                        ])),
                    ),
                    MenuItem::new("005", Some(Box::new(|| {})), None),
                ]),
            );

            assert_eq!(status_item.title(), "000");

            let menu = status_item.menu();
            assert_eq!(menu.items().len(), 3);
            assert_eq!(menu.inner.numberOfItems(), 3);

            assert_eq!(menu.items().get(0).unwrap().title(), "001");
            assert_eq!(
                menu.items().get(0).unwrap().inner.title(),
                NSString::from_str("001")
            );

            let menu_item = menu.items().get(0).unwrap();
            assert_eq!(menu_item.inner, menu.inner.itemAtIndex(0).unwrap());
            assert!(menu_item.callback.is_none());
            assert!(menu_item.inner.action().is_none());
            assert!(menu_item.submenu().is_none());

            let menu_item = menu.items().get(1).unwrap();
            assert!(menu_item.callback.is_none());
            assert!(!menu_item.inner.action().is_none()); // has Sel(submenuAction:)
            assert_eq!(
                menu_item.submenu().unwrap().inner,
                menu_item.inner.submenu().unwrap()
            );
            assert_eq!(menu_item.submenu().unwrap().items().len(), 2);
            assert_eq!(menu_item.submenu().unwrap().inner.numberOfItems(), 2);

            let menu_item = menu.items().get(2).unwrap();
            assert!(menu_item.submenu().is_none());
            assert!(!menu_item.inner.action().is_none());
            assert_eq!(
                menu_item.callback.as_ref().unwrap().inner.as_ref() as *const _,
                Id::cast::<STBMenuItemCallback>(menu_item.inner.target().unwrap()).as_ref()
                    as *const _,
            );
        }
    }

    #[test]
    fn reset_menu() {
        unsafe {
            let first_menu = Menu::new(vec![]);
            let first_menu_inner = first_menu.inner.clone();
            assert_eq!(
                {
                    let c: usize = msg_send![&first_menu_inner, retainCount];
                    c
                },
                2
            );

            let mut status_item = StatusItem::new_impl(NSStatusItem::new(), "000", first_menu);

            assert_eq!(status_item.inner.menu().unwrap(), first_menu_inner);
            assert!(
                2 < {
                    let c: usize = msg_send![&first_menu_inner, retainCount];
                    c
                }
            );

            let second_menu = Menu::new(vec![]);
            let second_menu_inner = second_menu.inner.clone();
            assert_eq!(
                {
                    let c: usize = msg_send![&second_menu_inner, retainCount];
                    c
                },
                2
            );

            status_item.set_menu(second_menu);
            assert_eq!(status_item.inner.menu().unwrap(), second_menu_inner);
            assert!(
                2 < {
                    let c: usize = msg_send![&second_menu_inner, retainCount];
                    c
                }
            );
            assert_eq!(
                {
                    let c: usize = msg_send![&first_menu_inner, retainCount];
                    c
                },
                1
            );
        }
    }

    #[test]
    fn reset_title() {
        unsafe {
            let mut status_item =
                StatusItem::new_impl(NSStatusItem::new(), "000", Menu::new(vec![]));
            assert_eq!(status_item.title(), "000");

            status_item.set_title("001");
            assert_eq!(status_item.title(), "001");
        }
    }

    #[test]
    fn click_menu() {
        unsafe {
            let click_count = Rc::new(Cell::new(0));
            let status_item = {
                let click_count = click_count.clone();
                let status_item = StatusItem::new_impl(
                    NSStatusItem::new(),
                    "000",
                    Menu::new(vec![MenuItem::new(
                        "001",
                        Some(Box::new(move || {
                            let c = click_count.get();
                            click_count.set(c + 1);
                        })),
                        None,
                    )]),
                );
                status_item
            };
            let menu_item_inner = status_item.inner.menu().unwrap().itemAtIndex(0).unwrap();

            assert_eq!(menu_item_inner.action().unwrap(), sel!(call:));

            assert_eq!(click_count.get(), 0);
            let _: () =
                msg_send![&menu_item_inner.target().unwrap(), call:menu_item_inner.as_ref()];
            assert_eq!(click_count.get(), 1);
            let _: () =
                msg_send![&menu_item_inner.target().unwrap(), call:menu_item_inner.as_ref()];
            assert_eq!(click_count.get(), 2);
        }
    }

    #[derive(Default)]
    pub struct EventLoopTestCounter {
        called_finish_launching: u32,
        called_update_windows: u32,
        called_next_event: u32,
        called_send_event: u32,
        called_sleep: u32,
    }

    #[test]
    fn event_loop() {
        thread_local!(
            pub static COUNTER: RefCell<EventLoopTestCounter> = RefCell::new(Default::default());
        );

        // dummy
        struct NSApplication {}
        impl NSApplication {
            #[allow(non_snake_case)]
            fn sharedApplication() -> Self {
                Self {}
            }
            #[allow(non_snake_case)]
            fn finishLaunching(&self) {
                COUNTER.with(|counter| {
                    counter.borrow_mut().called_finish_launching += 1;
                });
            }
            #[allow(non_snake_case)]
            fn updateWindows(&self) {
                COUNTER.with(|counter| {
                    counter.borrow_mut().called_update_windows += 1;
                });
            }
            #[allow(non_snake_case)]
            fn nextEventMatchingMask_untilDate_inMode_dequeue(
                &self,
                _: u64,
                _: Option<Id<NSDate>>,
                _: &NSString,
                _: bool,
            ) -> Option<Id<NSEvent>> {
                COUNTER.with(|counter| {
                    counter.borrow_mut().called_next_event += 1;
                });
                unsafe { Some(NSEvent::new()) }
            }
            #[allow(non_snake_case)]
            fn sendEvent(&self, _: &NSEvent) {
                COUNTER.with(|counter| {
                    counter.borrow_mut().called_send_event += 1;
                });
            }
        }
        let sleep_dummy = || {
            COUNTER.with(|counter| {
                counter.borrow_mut().called_sleep += 1;
            });
        };

        let (terminator, terminatee) = LoopTerminator::new();

        spawn(move || {
            sleep(Duration::from_millis(50));
            terminator.terminate();
        });

        event_loop!(terminatee, sleep_dummy(), ());

        COUNTER.with(|counter| {
            let counter = counter.borrow();
            assert_eq!(counter.called_finish_launching, 1);
            assert!(1 < counter.called_update_windows);
            assert!(1 < counter.called_next_event);
            assert!(1 < counter.called_send_event);
            assert!(1 < counter.called_sleep);
        });
    }

    #[test]
    fn loop_terminator_dropped() {
        // dummy
        struct NSApplication {}
        impl NSApplication {
            #[allow(non_snake_case)]
            fn sharedApplication() -> Self {
                Self {}
            }
            #[allow(non_snake_case)]
            fn finishLaunching(&self) {}
            #[allow(non_snake_case)]
            fn updateWindows(&self) {}
            #[allow(non_snake_case)]
            fn nextEventMatchingMask_untilDate_inMode_dequeue(
                &self,
                _: u64,
                _: Option<Id<NSDate>>,
                _: &NSString,
                _: bool,
            ) -> Option<Id<NSEvent>> {
                unsafe { Some(NSEvent::new()) }
            }
            #[allow(non_snake_case)]
            fn sendEvent(&self, _: &NSEvent) {}
        }
        let sleep_dummy = || {};

        // explicitly drop loop terminator
        let (_, terminatee) = LoopTerminator::new();

        assert_eq!(
            terminatee.receiver.try_recv(),
            Err(TryRecvError::Disconnected)
        );
        event_loop!(terminatee, sleep_dummy(), ());
    }
}

pub fn ns_alert(title: impl AsRef<str>, message: impl AsRef<str>) {
    let title = title.as_ref();
    let message = message.as_ref();
    unsafe {
        let alert = NSAlert::new();
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));

        let img = &NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("exclamationmark.triangle.fill"),
            Some(&NSString::from_str("exclamationmark.triangle.fill icon")),
        )
        .expect("an NSImage from the given system symbol name");

        alert.setIcon(Some(img));
        alert.runModal();
    }
}
