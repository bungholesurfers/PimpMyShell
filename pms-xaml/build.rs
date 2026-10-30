//! Generate a windows-rs projection for the bits of Windows.UI.Xaml we use. windows-rs stopped
//! shipping Windows.UI.Xaml, but its default metadata still describes it; everything outside
//! XAML is referenced from the `windows` crate instead of generated.

const TYPES: &[&str] = &[
    "Windows.UI.Xaml.Hosting.ElementCompositionPreview",
    "Windows.UI.Xaml.DependencyObject",
    "Windows.UI.Xaml.UIElement",
    "Windows.UI.Xaml.FrameworkElement",
    "Windows.UI.Xaml.Visibility",
    "Windows.UI.Xaml.Media.GeneralTransform",
    "Windows.UI.Xaml.Automation.AutomationProperties",
    "Windows.UI.Xaml.Thickness",
    "Windows.UI.Xaml.HorizontalAlignment",
    "Windows.UI.Xaml.VerticalAlignment",
    "Windows.UI.Xaml.TextAlignment",
    "Windows.UI.Xaml.Controls.Panel",
    "Windows.UI.Xaml.Controls.Canvas",
    "Windows.UI.Xaml.Controls.StackPanel",
    "Windows.UI.Xaml.Controls.Grid",
    "Windows.UI.Xaml.Controls.TextBlock",
    "Windows.UI.Xaml.Controls.UIElementCollection",
    "Windows.UI.Xaml.Controls.Orientation",
    "Windows.UI.Xaml.Shapes.Shape",
    "Windows.UI.Xaml.Shapes.Rectangle",
    "Windows.UI.Xaml.Media.Brush",
    "Windows.UI.Xaml.Media.SolidColorBrush",
    "Windows.UI.Xaml.Media.LoadedImageSurface",
    "Windows.UI.Xaml.Controls.Border",
    "Windows.UI.Xaml.CornerRadius",
    "Windows.UI.Xaml.TextTrimming",
    "Windows.UI.Xaml.TextWrapping",
    "Windows.UI.Xaml.RoutedEventArgs",
    "Windows.UI.Xaml.Input.TappedEventHandler",
    "Windows.UI.Xaml.Input.TappedRoutedEventArgs",
    "Windows.UI.Xaml.Input.PointerEventHandler",
    "Windows.UI.Xaml.Input.PointerRoutedEventArgs",
    "Windows.UI.Xaml.Controls.ColumnDefinition",
    "Windows.UI.Xaml.Controls.ColumnDefinitionCollection",
    "Windows.UI.Xaml.GridLength",
    "Windows.UI.Xaml.GridUnitType",
    "Windows.UI.Xaml.Media.FontFamily",
    "Windows.UI.Xaml.LineStackingStrategy",
    "Windows.UI.Xaml.Shapes.Path",
    "Windows.UI.Xaml.Shapes.Ellipse",
    "Windows.UI.Xaml.Media.Geometry",
    "Windows.UI.Xaml.Media.PathGeometry",
    "Windows.UI.Xaml.Media.PathFigure",
    "Windows.UI.Xaml.Media.PathFigureCollection",
    "Windows.UI.Xaml.Media.PathSegment",
    "Windows.UI.Xaml.Media.PathSegmentCollection",
    "Windows.UI.Xaml.Media.LineSegment",
    "Windows.UI.Xaml.Media.ArcSegment",
    "Windows.UI.Xaml.Media.SweepDirection",
    "Windows.UI.Xaml.Media.CompositionTarget",
    "Windows.UI.Xaml.Controls.Primitives.Popup",
    "Windows.UI.Xaml.XamlRoot",
    // The Start button's hover and press, for its glow (glow.rs).
    "Windows.UI.Xaml.Controls.Primitives.ButtonBase",
    "Windows.UI.Xaml.Controls.ContentControl",
    "Windows.UI.Xaml.Controls.Control",
    "Windows.UI.Xaml.Controls.Primitives.ToggleButton",
    // App buttons' icons, rendered to pixels for their colors.
    "Windows.UI.Xaml.Media.Imaging.RenderTargetBitmap",
    "Windows.UI.Xaml.Media.ImageSource",
    "Windows.UI.Xaml.Controls.Image",
    "Windows.UI.Xaml.DependencyProperty",
    "Windows.UI.Xaml.DependencyPropertyChangedCallback",
];

/// Namespaces the `windows` crate already projects (and which XAML signatures mention).
const FROM_WINDOWS_CRATE: &[&str] = &[
    "Windows.ApplicationModel",
    "Windows.Devices",
    "Windows.Foundation",
    "Windows.Graphics",
    "Windows.Media",
    "Windows.Storage",
    "Windows.System",
    "Windows.UI.Color",
    "Windows.UI.Composition",
    "Windows.UI.Core",
    "Windows.UI.Input",
    "Windows.UI.Text",
    "Windows.UI.UIContext",
    "Windows.UI.WindowManagement",
];

fn main() {
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("bindings.rs");
    let mut args = vec!["--out".to_string(), out.display().to_string(), "--flat".into(), "--no-allow".into(), "--filter".into()];
    args.extend(TYPES.iter().map(|s| s.to_string()));
    for ns in FROM_WINDOWS_CRATE {
        args.push("--reference".into());
        args.push(format!("windows,skip-root,{ns}"));
    }
    let warnings = windows_bindgen::bindgen(args);
    // Methods mentioning XAML types outside TYPES are skipped (one warning each); that is intended.
    let _ = warnings;
    println!("cargo::rerun-if-changed=build.rs");
}
