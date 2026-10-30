//! Hand-written bindings for the XAML Diagnostics COM interfaces (`xamlom.h` in the Windows SDK).
//! windows-rs doesn't ship these. Vtable order must match the header exactly.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code)]

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::RECT;
use windows_core::{BOOL, BSTR, GUID, HRESULT, IInspectable, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

pub type InstanceHandle = u64;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualMutationType(pub i32);
impl VisualMutationType {
    pub const Add: Self = Self(0);
    pub const Remove: Self = Self(1);
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualElementState(pub i32);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseValueSource(pub i32);

/// BSTRs in structs handed to us as `[in]` are owned by the caller, hence `ManuallyDrop`.
#[repr(C)]
pub struct SourceInfo {
    pub FileName: ManuallyDrop<BSTR>,
    pub LineNumber: u32,
    pub ColumnNumber: u32,
    pub CharPosition: u32,
    pub Hash: ManuallyDrop<BSTR>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ParentChildRelation {
    pub Parent: InstanceHandle,
    pub Child: InstanceHandle,
    pub ChildIndex: u32,
}

#[repr(C)]
pub struct VisualElement {
    pub Handle: InstanceHandle,
    pub SrcInfo: SourceInfo,
    pub Type: ManuallyDrop<BSTR>,
    pub Name: ManuallyDrop<BSTR>,
    pub NumChildren: u32,
}

#[repr(C)]
pub struct PropertyChainSource {
    pub Handle: InstanceHandle,
    pub TargetType: BSTR,
    pub Name: BSTR,
    pub Source: BaseValueSource,
    pub SrcInfo: SourceInfo,
}

#[repr(C)]
pub struct PropertyChainValue {
    pub Index: u32,
    pub Type: BSTR,
    pub DeclaringType: BSTR,
    pub ValueType: BSTR,
    pub ItemType: BSTR,
    pub Value: BSTR,
    pub Overridden: BOOL,
    pub MetadataBits: i64,
    pub PropertyName: BSTR,
    pub PropertyChainIndex: u32,
}

#[repr(C)]
pub struct CollectionElementValue {
    pub Index: u32,
    pub ValueType: BSTR,
    pub Value: BSTR,
    pub MetadataBits: i64,
}

#[interface("AA7A8931-80E4-4FEC-8F3B-553F87B4966E")]
pub unsafe trait IVisualTreeServiceCallback: IUnknown {
    pub fn OnVisualTreeChange(
        &self,
        relation: ParentChildRelation,
        element: VisualElement,
        mutation_type: VisualMutationType,
    ) -> HRESULT;
}

#[interface("BAD9EB88-AE77-4397-B948-5FA2DB0A19EA")]
pub unsafe trait IVisualTreeServiceCallback2: IVisualTreeServiceCallback {
    pub fn OnElementStateChanged(
        &self,
        element: InstanceHandle,
        element_state: VisualElementState,
        context: PCWSTR,
    ) -> HRESULT;
}

#[interface("A593B11A-D17F-48BB-8F66-83910731C8A5")]
pub unsafe trait IVisualTreeService: IUnknown {
    pub fn AdviseVisualTreeChange(&self, callback: *mut core::ffi::c_void) -> HRESULT;
    pub fn UnadviseVisualTreeChange(&self, callback: *mut core::ffi::c_void) -> HRESULT;
    pub fn GetEnums(&self, count: *mut u32, enums: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn CreateInstance(&self, type_name: *const u16, value: *const u16, out: *mut InstanceHandle) -> HRESULT;
    pub fn GetPropertyValuesChain(
        &self,
        handle: InstanceHandle,
        source_count: *mut u32,
        sources: *mut *mut PropertyChainSource,
        property_count: *mut u32,
        values: *mut *mut PropertyChainValue,
    ) -> HRESULT;
    pub fn SetProperty(&self, handle: InstanceHandle, value: InstanceHandle, property_index: u32) -> HRESULT;
    pub fn ClearProperty(&self, handle: InstanceHandle, property_index: u32) -> HRESULT;
    pub fn GetCollectionCount(&self, handle: InstanceHandle, size: *mut u32) -> HRESULT;
    pub fn GetCollectionElements(
        &self,
        handle: InstanceHandle,
        start_index: u32,
        element_count: *mut u32,
        values: *mut *mut CollectionElementValue,
    ) -> HRESULT;
    pub fn AddChild(&self, parent: InstanceHandle, child: InstanceHandle, index: u32) -> HRESULT;
    pub fn RemoveChild(&self, parent: InstanceHandle, index: u32) -> HRESULT;
    pub fn ClearChildren(&self, parent: InstanceHandle) -> HRESULT;
}

#[interface("130F5136-EC43-4F61-89C7-9801A36D2E95")]
pub unsafe trait IVisualTreeService2: IVisualTreeService {
    pub fn GetPropertyIndex(&self, object: InstanceHandle, property_name: PCWSTR, index: *mut u32) -> HRESULT;
    pub fn GetProperty(&self, object: InstanceHandle, property_index: u32, value: *mut InstanceHandle) -> HRESULT;
    pub fn ReplaceResource(&self, dictionary: InstanceHandle, key: InstanceHandle, new_value: InstanceHandle) -> HRESULT;
    pub fn RenderTargetBitmap(
        &self,
        handle: InstanceHandle,
        options: i32,
        max_width: u32,
        max_height: u32,
        bitmap: *mut *mut core::ffi::c_void,
    ) -> HRESULT;
}

#[interface("0E79C6E0-85A0-4BE8-B41A-655CF1FD19BD")]
pub unsafe trait IVisualTreeService3: IVisualTreeService2 {
    pub fn ResolveResource(
        &self,
        context: InstanceHandle,
        resource_name: PCWSTR,
        resource_type: i32,
        property_index: u32,
    ) -> HRESULT;
    pub fn GetDictionaryItem(
        &self,
        dictionary: InstanceHandle,
        resource_name: PCWSTR,
        is_implicit_style: BOOL,
        out: *mut InstanceHandle,
    ) -> HRESULT;
    pub fn AddDictionaryItem(&self, dictionary: InstanceHandle, key: InstanceHandle, value: InstanceHandle) -> HRESULT;
    pub fn RemoveDictionaryItem(&self, dictionary: InstanceHandle, key: InstanceHandle) -> HRESULT;
}

#[interface("18C9E2B6-3F43-4116-9F2B-FF935D7770D2")]
pub unsafe trait IXamlDiagnostics: IUnknown {
    pub fn GetDispatcher(&self, out: *mut Option<IInspectable>) -> HRESULT;
    pub fn GetUiLayer(&self, out: *mut Option<IInspectable>) -> HRESULT;
    pub fn GetApplication(&self, out: *mut Option<IInspectable>) -> HRESULT;
    pub fn GetIInspectableFromHandle(&self, handle: InstanceHandle, out: *mut Option<IInspectable>) -> HRESULT;
    pub fn GetHandleFromIInspectable(&self, instance: *mut core::ffi::c_void, out: *mut InstanceHandle) -> HRESULT;
    pub fn HitTest(&self, rect: RECT, count: *mut u32, handles: *mut *mut InstanceHandle) -> HRESULT;
    pub fn RegisterInstance(&self, instance: *mut core::ffi::c_void, out: *mut InstanceHandle) -> HRESULT;
    pub fn GetInitializationData(&self, out: *mut BSTR) -> HRESULT;
}

/// `HRESULT InitializeXamlDiagnosticsEx(endPointName, pid, wszDllXamlDiagnostics, wszTAPDllName, tapClsid, wszInitializationData)`
/// exported by Windows.UI.Xaml.dll (system XAML) and Microsoft.UI.Xaml.dll (WinUI 3).
pub type InitializeXamlDiagnosticsExFn =
    unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, GUID, PCWSTR) -> HRESULT;
