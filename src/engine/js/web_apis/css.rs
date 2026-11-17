//! `CSS` global object — `CSS.supports()` and `CSS.escape()`.
//!
//! Provides the CSSOM CSS interface exposed on the global scope.

use pixi_byte::value::jsobject::{JSObject, Property};
use pixi_byte::vm::VM;
use pixi_byte::{JSResult, JSValue};
use std::cell::RefCell;
use std::rc::Rc;

/// Installs the `CSS` global object and `CSSStyleDeclaration` constructor.
pub(crate) fn install_css(engine: &mut pixi_byte::JSEngine) {
    let global = engine.global_mut();

    // CSSStyleDeclaration.prototype — shared by style and computed-style wrappers.
    let csd_proto = Rc::new(RefCell::new(JSObject::new()));
    csd_proto.borrow_mut().set(
        "constructor".to_string(),
        JSValue::from_object(Rc::clone(&csd_constructor_stub())),
    );

    // CSSStyleDeclaration() constructor — `new CSSStyleDeclaration()` creates
    // an empty mutable declaration block.
    let mut csd_constructor = JSObject::new();
    csd_constructor.set(
        "__call__".to_string(),
        JSValue::from_native_function(css_style_declaration_construct),
    );
    csd_constructor.set(
        "__construct__".to_string(),
        JSValue::from_native_function(css_style_declaration_construct),
    );
    csd_constructor.define_property(
        "prototype".to_string(),
        Property::read_only(JSValue::from_object(Rc::clone(&csd_proto))),
    );

    // CSS global object.
    let mut css = JSObject::new();
    css.set(
        "supports".to_string(),
        JSValue::from_native_function(css_supports),
    );
    css.set(
        "escape".to_string(),
        JSValue::from_native_function(css_escape),
    );

    let mut global = global.borrow_mut();
    global.set(
        "CSSStyleDeclaration".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(csd_constructor))),
    );
    global.set(
        "CSS".to_string(),
        JSValue::from_object(Rc::new(RefCell::new(css))),
    );
}

/// Stub prototype constructor for `CSSStyleDeclaration.prototype.constructor`.
fn csd_constructor_stub() -> Rc<RefCell<JSObject>> {
    let mut obj = JSObject::new();
    obj.set(
        "__call__".to_string(),
        JSValue::from_native_function(css_style_declaration_construct),
    );
    obj.set(
        "__construct__".to_string(),
        JSValue::from_native_function(css_style_declaration_construct),
    );
    Rc::new(RefCell::new(obj))
}

/// `new CSSStyleDeclaration()` — creates an empty declaration block.
fn css_style_declaration_construct(_vm: &mut VM, _args: Vec<JSValue>) -> JSResult<JSValue> {
    Ok(JSValue::undefined())
}

/// `CSS.supports(property, value)` or `CSS.supports(conditionText)`.
///
/// Returns `true` when the implementation supports the given property-value
/// pair.  A full CSS parser is not available, so validation is conservative:
/// only well-known properties with simple values are recognised; everything
/// else reports `false`.
fn css_supports(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let arg = args.get(1).cloned().unwrap_or(JSValue::undefined());
    let text = match arg.as_string() {
        Some(t) => t,
        None => return Ok(JSValue::from_bool(false)),
    };

    // `CSS.supports("display: flex")` — single condition-text form.
    if text.contains(':') {
        return Ok(JSValue::from_bool(supports_condition_text(&text)));
    }

    // `CSS.supports(property, value)` — two-argument form.
    let value = args.get(2).cloned().unwrap_or(JSValue::undefined());
    let value = match value.as_string() {
        Some(v) => v,
        None => return Ok(JSValue::from_bool(false)),
    };
    Ok(JSValue::from_bool(supports_property_value(&text, &value)))
}

/// `CSS.supports(conditionText)` — parses "property: value" and checks.
fn supports_condition_text(text: &str) -> bool {
    let mut parts = text.splitn(2, ':');
    let prop = parts.next().unwrap_or("").trim();
    let value = parts.next().unwrap_or("").trim();
    if prop.is_empty() || value.is_empty() {
        return false;
    }
    supports_property_value(prop, value)
}

/// `CSS.supports(property, value)` — conservative property/value validation.
fn supports_property_value(property: &str, value: &str) -> bool {
    let prop = property.trim().to_ascii_lowercase();
    let val = value.trim();

    match prop.as_str() {
        "display" => matches!(
            val,
            "none"
                | "block"
                | "inline"
                | "inline-block"
                | "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "contents"
                | "run-in"
        ),
        "color" => is_css_color(val),
        "background" => true, // shorthand, broadly supported
        "background-color" => is_css_color(val),
        "width" => is_css_dimension(val),
        "height" => is_css_dimension(val),
        "margin" | "padding" | "border" => true, // shorthands
        "font" | "font-family" | "font-size" | "font-weight" | "font-style" => true,
        "text-align" => matches!(
            val,
            "left" | "right" | "center" | "justify" | "start" | "end"
        ),
        "visibility" => matches!(val, "visible" | "hidden" | "collapse"),
        "position" => matches!(val, "static" | "relative" | "absolute" | "fixed" | "sticky"),
        "float" => matches!(
            val,
            "none" | "left" | "right" | "inline-start" | "inline-end"
        ),
        "clear" => matches!(
            val,
            "none" | "left" | "right" | "both" | "inline-start" | "inline-end"
        ),
        "overflow" => matches!(val, "visible" | "hidden" | "scroll" | "auto"),
        "opacity" => is_css_number(val),
        "z-index" => is_css_integer(val) || val == "auto",
        _ => false,
    }
}

/// `CSS.escape()` — escapes a string for use in a CSS selector.
fn css_escape(_vm: &mut VM, args: Vec<JSValue>) -> JSResult<JSValue> {
    let s = args.get(1).and_then(JSValue::as_string).unwrap_or_default();
    let escaped = escape_identifier(s);
    Ok(JSValue::from_string(escaped))
}

/// Escapes a string for use as a CSS identifier / selector fragment.
fn escape_identifier(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            // Characters that must be escaped in CSS identifiers.
            c if c.is_ascii_whitespace() => {
                out.push('\\');
                out.push_str(&format!("{:X} ", c as u32));
            }
            c if c == '.'
                || c == '#'
                || c == ':'
                || c == '['
                || c == ']'
                || c == '('
                || c == ')'
                || c == ','
                || c == '+'
                || c == '~'
                || c == '>'
                || c == '*'
                || c == '!'
                || c == '@'
                || c == '%'
                || c == '^'
                || c == '&'
                || c == '`'
                || c == '|'
                || c == '='
                || c == '"'
                || c == '\''
                || c == '/'
                || c == '{'
                || c == '}'
                || c == '<'
                || c == '>'
                || c == '?'
                || c == ';' =>
            {
                out.push('\\');
                out.push(c);
            }
            // Control characters and non-ASCII are hex-escaped.
            c if c.is_control() || (c as u32) > 0x7E => {
                out.push('\\');
                out.push_str(&format!("{:X} ", c as u32));
            }
            c => out.push(c),
        }
    }
    // Remove trailing space added by hex escapes.
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Whether `value` looks like a CSS color (keyword, hex, rgb(), etc.).
fn is_css_color(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() {
        return false;
    }
    // Hex colors: #rgb, #rgba, #rrggbb, #rrggbbaa
    if v.starts_with('#') {
        let rest = &v[1..];
        return !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_hexdigit())
            && matches!(rest.len(), 3 | 4 | 6 | 8);
    }
    // Functional colors: rgb(), rgba(), hsl(), hsla(), hwb(), lab(), lch(), color()
    if v.ends_with(')')
        && (v.starts_with("rgb(")
            || v.starts_with("rgba(")
            || v.starts_with("hsl(")
            || v.starts_with("hsla(")
            || v.starts_with("hwb(")
            || v.starts_with("lab(")
            || v.starts_with("lch(")
            || v.starts_with("color("))
    {
        return true;
    }
    // Named keywords (subset).
    matches!(
        v,
        "transparent"
            | "currentColor"
            | "currentcolor"
            | "inherit"
            | "initial"
            | "unset"
            | "revert"
            | "black"
            | "silver"
            | "gray"
            | "grey"
            | "white"
            | "maroon"
            | "red"
            | "purple"
            | "fuchsia"
            | "green"
            | "lime"
            | "olive"
            | "yellow"
            | "navy"
            | "blue"
            | "teal"
            | "aqua"
            | "orange"
            | "aliceblue"
            | "antiquewhite"
            | "aquamarine"
            | "azure"
            | "beige"
            | "bisque"
            | "blanchedalmond"
            | "blueviolet"
            | "brown"
            | "burlywood"
            | "cadetblue"
            | "chartreuse"
            | "chocolate"
            | "coral"
            | "cornflowerblue"
            | "cornsilk"
            | "crimson"
            | "cyan"
            | "darkblue"
            | "darkcyan"
            | "darkgoldenrod"
            | "darkgray"
            | "darkgreen"
            | "darkgrey"
            | "darkkhaki"
            | "darkmagenta"
            | "darkolivegreen"
            | "darkorange"
            | "darkorchid"
            | "darkred"
            | "darksalmon"
            | "darkseagreen"
            | "darkslateblue"
            | "darkslategray"
            | "darkslategrey"
            | "darkturquoise"
            | "darkviolet"
            | "deeppink"
            | "deepskyblue"
            | "dimgray"
            | "dimgrey"
            | "dodgerblue"
            | "firebrick"
            | "floralwhite"
            | "forestgreen"
            | "gainsboro"
            | "ghostwhite"
            | "gold"
            | "goldenrod"
            | "greenyellow"
            | "honeydew"
            | "hotpink"
            | "indianred"
            | "indigo"
            | "ivory"
            | "khaki"
            | "lavender"
            | "lavenderblush"
            | "lawngreen"
            | "lemonchiffon"
            | "lightblue"
            | "lightcoral"
            | "lightcyan"
            | "lightgoldenrodyellow"
            | "lightgray"
            | "lightgreen"
            | "lightgrey"
            | "lightpink"
            | "lightsalmon"
            | "lightseagreen"
            | "lightskyblue"
            | "lightslategray"
            | "lightslategrey"
            | "lightsteelblue"
            | "lightyellow"
            | "limegreen"
            | "linen"
            | "magenta"
            | "mediumaquamarine"
            | "mediumblue"
            | "mediumorchid"
            | "mediumpurple"
            | "mediumseagreen"
            | "mediumslateblue"
            | "mediumspringgreen"
            | "mediumturquoise"
            | "mediumvioletred"
            | "midnightblue"
            | "mintcream"
            | "mistyrose"
            | "moccasin"
            | "navajowhite"
            | "oldlace"
            | "olivedrab"
            | "orangered"
            | "orchid"
            | "palegoldenrod"
            | "palegreen"
            | "paleturquoise"
            | "palevioletred"
            | "papayawhip"
            | "peachpuff"
            | "peru"
            | "pink"
            | "plum"
            | "powderblue"
            | "rosybrown"
            | "royalblue"
            | "saddlebrown"
            | "salmon"
            | "sandybrown"
            | "seagreen"
            | "seashell"
            | "sienna"
            | "skyblue"
            | "slateblue"
            | "slategray"
            | "slategrey"
            | "snow"
            | "springgreen"
            | "steelblue"
            | "tan"
            | "thistle"
            | "tomato"
            | "turquoise"
            | "violet"
            | "wheat"
            | "whitesmoke"
            | "yellowgreen"
            | "rebeccapurple"
    )
}

/// Whether `value` looks like a CSS dimension (with unit or zero).
fn is_css_dimension(value: &str) -> bool {
    let v = value.trim();
    if v == "0" {
        return true;
    }
    // Must end with a unit (px, em, rem, %, etc.) or be a pure number.
    let num_end = v
        .chars()
        .position(|c| !c.is_ascii_digit() && c != '.' && c != '-');
    match num_end {
        Some(i) => {
            let unit = &v[i..];
            !unit.is_empty()
                && unit.chars().all(|c| c.is_ascii_alphabetic() || c == '%')
                && v[..i].parse::<f64>().is_ok()
        }
        None => v.parse::<f64>().is_ok(),
    }
}

/// Whether `value` is a CSS number (integer or decimal).
fn is_css_number(value: &str) -> bool {
    let v = value.trim();
    v.parse::<f64>().is_ok()
}

/// Whether `value` is a CSS integer.
fn is_css_integer(value: &str) -> bool {
    let v = value.trim();
    v.parse::<i64>().is_ok()
}
