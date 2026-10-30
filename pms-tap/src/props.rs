//! Effective property values via `GetPropertyValuesChain`. Must run on the XAML UI thread.

use std::fmt::Write as _;

use windows::Win32::System::Com::CoTaskMemFree;

use crate::xamlom::*;

const BASE_VALUE_SOURCES: &[&str] = &[
    "Unknown", "Default", "BuiltInStyle", "Style", "Local", "Inherited", "DefaultStyleTrigger",
    "TemplateTrigger", "StyleTrigger", "ImplicitStyleReference", "ParentTemplate",
    "ParentTemplateTrigger", "Animation", "Coercion", "VisualState",
];
const SOURCE_DEFAULT: i32 = 1;

pub const IS_VALUE_HANDLE: i64 = 0x1;
pub const IS_VALUE_NULL: i64 = 0x20;

/// One effective (non-overridden) property value.
pub struct Prop {
    /// Property index as `SetProperty`/`ClearProperty` want it.
    pub index: u32,
    pub name: String,
    pub value_type: String,
    pub declaring_type: String,
    pub value: String,
    pub source: i32,
    pub metadata: i64,
}

impl Prop {
    pub fn is_null(&self) -> bool {
        self.metadata & IS_VALUE_NULL != 0
    }
    pub fn is_handle(&self) -> bool {
        self.metadata & IS_VALUE_HANDLE != 0
    }
}

pub fn effective_props(service: &IVisualTreeService3, handle: InstanceHandle) -> windows_core::Result<Vec<Prop>> {
    unsafe fn take<T>(ptr: *mut T, n: u32) -> Vec<T> {
        if ptr.is_null() {
            return Vec::new();
        }
        let v = (0..n as usize).map(|i| unsafe { ptr.add(i).read() }).collect();
        unsafe { CoTaskMemFree(Some(ptr as _)) };
        v
    }
    let (mut ns, mut sources, mut nv, mut values) = (0u32, std::ptr::null_mut(), 0u32, std::ptr::null_mut());
    let (sources, values): (Vec<PropertyChainSource>, Vec<PropertyChainValue>) = unsafe {
        service.GetPropertyValuesChain(handle, &mut ns, &mut sources, &mut nv, &mut values).ok()?;
        (take(sources, ns), take(values, nv))
    };
    let mut props: Vec<Prop> = values
        .iter()
        .filter(|v| !v.Overridden.as_bool())
        .map(|v| Prop {
            index: v.Index,
            name: v.PropertyName.to_string(),
            value_type: v.ValueType.to_string(),
            declaring_type: v.DeclaringType.to_string(),
            value: v.Value.to_string(),
            source: sources.get(v.PropertyChainIndex as usize).map_or(0, |s| s.Source.0),
            metadata: v.MetadataBits,
        })
        .collect();
    props.sort_by(|a, b| a.name.cmp(&b.name));
    props.dedup_by(|a, b| a.name == b.name && a.declaring_type == b.declaring_type);
    Ok(props)
}

/// Without a filter, only non-default values are shown (the default-valued list is enormous).
pub fn dump(service: &IVisualTreeService3, handle: InstanceHandle, filter: Option<&str>) -> windows_core::Result<String> {
    let filter = filter.map(str::to_ascii_lowercase);
    let mut out = String::new();
    for p in effective_props(service, handle)? {
        let shown = match &filter {
            Some(f) => p.name.to_ascii_lowercase().contains(f),
            None => p.source != SOURCE_DEFAULT,
        };
        if !shown {
            continue;
        }
        let value = if p.is_null() {
            "null".to_string()
        } else if p.is_handle() {
            format!("<object {:#x}>", p.value.parse::<u64>().unwrap_or(0))
        } else {
            p.value.clone()
        };
        let src = BASE_VALUE_SOURCES.get(p.source as usize).copied().unwrap_or("?");
        let _ = writeln!(out, "{} : {} = {}   [{src}]", p.name, p.value_type, value);
    }
    Ok(out)
}

pub fn find_prop(service: &IVisualTreeService3, handle: InstanceHandle, name: &str) -> windows_core::Result<Option<Prop>> {
    Ok(effective_props(service, handle)?.into_iter().find(|p| p.name == name))
}
