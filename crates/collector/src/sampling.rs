use crate::{
    native,
    netvars::{self, PlayerOffsets},
    STOP,
};
use core::ffi::c_void;
use gmod_dglab_common::{
    queue::EventQueue,
    tracker::{GameEvent, PlayerSample, Tracker},
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        LazyLock, Mutex, OnceLock,
    },
    time::Instant,
};

pub static EVENTS: LazyLock<EventQueue> = LazyLock::new(EventQueue::default);
pub static GENERATION: AtomicU64 = AtomicU64::new(0);
static BINDINGS: OnceLock<Bindings> = OnceLock::new();
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static STATE: LazyLock<Mutex<SamplingState>> = LazyLock::new(|| {
    Mutex::new(SamplingState {
        tracker: Tracker::default(),
        last: None,
        max: None,
        generation: 0,
        initialized: false,
    })
});
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static ERRORS: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static MAX_MICROS: AtomicU64 = AtomicU64::new(0);
static DISABLED: AtomicBool = AtomicBool::new(false);
static ERROR_DETAIL: OnceLock<String> = OnceLock::new();

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, protection: u32, old: *mut u32) -> i32;
    fn GetCurrentThreadId() -> u32;
}
type FrameFn = unsafe extern "system" fn(*mut c_void, i32);
struct SamplingState {
    tracker: Tracker,
    last: Option<PlayerSample>,
    max: Option<i32>,
    generation: u64,
    initialized: bool,
}
struct Bindings {
    engine: usize,
    entities: usize,
    entity_fn: usize,
    local_fn: usize,
    in_game_fn: usize,
    offsets: PlayerOffsets,
}

fn pointer(address: usize) -> io::Result<usize> {
    native::read::<8>(address)
        .map(usize::from_le_bytes)
        .ok_or_else(|| io::Error::other("原生指针不可读"))
}
fn method(object: usize, index: usize, expected: usize) -> io::Result<usize> {
    let table = pointer(object)?;
    let address = pointer(
        table
            .checked_add(index * 8)
            .ok_or_else(|| io::Error::other("虚表地址溢出"))?,
    )?;
    if address != expected {
        return Err(io::Error::other("虚函数已被修改或版本不符，拒绝接入"));
    }
    Ok(address)
}
impl Bindings {
    fn prepare() -> io::Result<Self> {
        let engine = native::base("engine.dll")?;
        let client = native::base("client.dll")?;
        let engine_object = engine + 0x4993c0;
        let entities = client + 0x9df7a8;
        Ok(Self {
            engine: engine_object,
            entities,
            local_fn: method(engine_object, 12, engine + 0x795e0)?,
            in_game_fn: method(engine_object, 26, engine + 0x79af0)?,
            entity_fn: method(entities, 3, client + 0x20ab00)?,
            offsets: netvars::resolve_player(client)?,
        })
    }
    fn connected(&self) -> bool {
        // SAFETY: 仅从已验证的游戏帧回调调用，目标版本函数返回规范 bool 字节。
        let call: unsafe extern "system" fn(*mut c_void) -> u8 =
            unsafe { std::mem::transmute(self.in_game_fn) };
        unsafe { call(self.engine as *mut c_void) == 1 }
    }
    fn sample(&self) -> io::Result<(Option<PlayerSample>, Option<i32>)> {
        if !self.connected() {
            return Ok((None, None));
        }
        let local: unsafe extern "system" fn(*mut c_void) -> i32 =
            unsafe { std::mem::transmute(self.local_fn) };
        // SAFETY: 当前版本方法身份和 ABI 已核对，在原回调返回后的游戏线程调用。
        let index = unsafe { local(self.engine as *mut c_void) };
        if !(1..=8191).contains(&index) {
            return Ok((None, None));
        }
        let entity: unsafe extern "system" fn(*mut c_void, i32) -> *mut c_void =
            unsafe { std::mem::transmute(self.entity_fn) };
        let object = unsafe { entity(self.entities as *mut c_void, index) } as usize;
        if object == 0 {
            return Ok((None, None));
        }
        // 来自本版本 GetClientEntity 的 (index - 0x4001) * 32 槽位访问；同时核对实际对象基址。
        let record = self
            .entities
            .checked_sub(0x80020)
            .and_then(|base| base.checked_add(index as usize * 32))
            .ok_or_else(|| io::Error::other("实体记录地址无效"))?;
        let data = native::read::<16>(record).ok_or_else(|| io::Error::other("实体记录不可读"))?;
        if usize::from_le_bytes(data[..8].try_into().map_err(io::Error::other)?) != object {
            return Err(io::Error::other("实体子对象基址尚未验证"));
        }
        let serial = u32::from_le_bytes(data[8..12].try_into().map_err(io::Error::other)?);
        let integer = |offset: usize| {
            object
                .checked_add(offset)
                .and_then(native::read::<4>)
                .map(i32::from_le_bytes)
                .ok_or_else(|| io::Error::other("玩家字段不可读"))
        };
        let life_state = object
            .checked_add(self.offsets.life_state)
            .and_then(native::read::<1>)
            .ok_or_else(|| io::Error::other("生命状态不可读"))?[0];
        if life_state > 4 {
            return Err(io::Error::other("生命状态超出已支持范围"));
        }
        let health = integer(self.offsets.health)?;
        let max = self
            .offsets
            .max_health
            .map(integer)
            .transpose()?
            .filter(|n| *n > 0);
        let armor = self
            .offsets
            .armor
            .map(integer)
            .transpose()?
            .filter(|n| *n >= 0);
        Ok((
            Some(PlayerSample {
                entity_key: ((serial as u64) << 32) | index as u64,
                health,
                armor,
                life_state,
            }),
            max,
        ))
    }
}

unsafe extern "system" fn frame(this: *mut c_void, stage: i32) {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    let original: FrameFn = unsafe { std::mem::transmute(original) };
    // 原始游戏逻辑始终先执行，回调只在网络更新结束后采样。
    unsafe { original(this, stage) };
    if !STOP.load(Ordering::Acquire) && !DISABLED.load(Ordering::Acquire) {
        if let Some(bindings) = BINDINGS.get() {
            if stage == 4 || (stage == 0 && !bindings.connected()) {
                let start = Instant::now();
                let value = if stage == 4 {
                    bindings.sample()
                } else {
                    Ok((None, None))
                };
                if let Ok(mut state) = STATE.try_lock() {
                    let generation = GENERATION.load(Ordering::Acquire);
                    if state.generation != generation {
                        state.tracker.reset();
                        state.initialized = false;
                        state.generation = generation;
                    }
                    let (sample, max) = match value {
                        Ok(value) => value,
                        Err(error) => {
                            ERRORS.fetch_add(1, Ordering::Relaxed);
                            let _ = ERROR_DETAIL.set(error.to_string());
                            DISABLED.store(true, Ordering::Release);
                            (None, None)
                        }
                    };
                    let events = state.tracker.observe(sample);
                    if !state.initialized || state.last != sample || state.max != max {
                        let thread_id = unsafe { GetCurrentThreadId() };
                        EVENTS.push_generation(
                            GameEvent::PlayerState {
                                sample,
                                max_health: max,
                                thread_id,
                            },
                            start,
                            generation,
                        );
                        state.last = sample;
                        state.max = max;
                        state.initialized = true;
                    }
                    for event in events {
                        EVENTS.push_generation(event, start, generation);
                    }
                }
                FRAMES.fetch_add(1, Ordering::Relaxed);
                MAX_MICROS.fetch_max(
                    start.elapsed().as_micros().min(u64::MAX as u128) as u64,
                    Ordering::Relaxed,
                );
            }
        }
    }
    IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
}

fn replace(slot: usize, expected: usize, value: usize) -> io::Result<()> {
    if !slot.is_multiple_of(8) {
        return Err(io::Error::other("虚表槽未对齐"));
    }
    let mut old = 0u32;
    // SAFETY: 槽位已经由当前构建的虚表验证；只修改一个对齐的指针，随后恢复页保护。
    if unsafe { VirtualProtect(slot as *mut c_void, 8, 4, &mut old) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let changed = unsafe { AtomicUsize::from_ptr(slot as *mut usize) }.compare_exchange(
        expected,
        value,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    let mut ignored = 0;
    let restored = unsafe { VirtualProtect(slot as *mut c_void, 8, old, &mut ignored) };
    if restored == 0 {
        return Err(io::Error::other("虚表页保护恢复失败，停止采样"));
    }
    changed
        .map(|_| ())
        .map_err(|_| io::Error::other("虚表槽在接入时变化，未覆盖其他回调"))
}
pub struct Hook {
    slot: usize,
    original: usize,
}
impl Hook {
    pub fn install() -> io::Result<Self> {
        LazyLock::force(&EVENTS);
        LazyLock::force(&STATE);
        let bindings = Bindings::prepare()?;
        let client = native::base("client.dll")?;
        let original = method(client + 0x87a6d0, 35, client + 0x204300)?;
        let table = pointer(client + 0x87a6d0)?;
        if table != client + 0x6b8608 {
            return Err(io::Error::other("客户端虚表不符"));
        }
        BINDINGS
            .set(bindings)
            .map_err(|_| io::Error::other("采样上下文已初始化"))?;
        ORIGINAL.store(original, Ordering::Release);
        let hook = Self {
            slot: table + 35 * 8,
            original,
        };
        if let Err(error) = replace(hook.slot, original, frame as *const () as usize) {
            STOP.store(true, Ordering::Release);
            return Err(error);
        }
        Ok(hook)
    }
    pub fn stop(&self) -> io::Result<()> {
        replace(self.slot, frame as *const () as usize, self.original)
    }
}
pub fn statistics() -> String {
    format!(
        "采样回调 {} 次，读取错误 {} 次，采样最大耗时 {} 微秒，队列丢弃 {} 条，在途回调 {}",
        FRAMES.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
        MAX_MICROS.load(Ordering::Relaxed),
        EVENTS.dropped(),
        IN_FLIGHT.load(Ordering::Relaxed)
    ) + &ERROR_DETAIL
        .get()
        .map(|error| format!("；采样已停用：{error}"))
        .unwrap_or_default()
}
