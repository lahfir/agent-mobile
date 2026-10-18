//! Shared validators: one rule per kind, the same faults on both surfaces.

use serde_json::{Value, json};

use super::{check, check_args, from_words, split_ref_text};
use crate::surface::{Arg, Kind, find};

fn arg(kind: Kind) -> Arg {
    Arg {
        name: "a",
        help: "help",
        kind,
        required: true,
        flag: false,
    }
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn point_words_become_a_ref_or_a_finite_point() {
    let point = arg(Kind::Point);
    let reference = from_words(Kind::Point, &words(&["@s:e1"]));
    assert_eq!(check(&point, &reference), Ok(json!({"ref": "@s:e1"})));
    let xy = from_words(Kind::Point, &words(&["10", "20.5"]));
    assert_eq!(check(&point, &xy), Ok(json!({"x": 10.0, "y": 20.5})));
    let bad = from_words(Kind::Point, &words(&["nan", "1"]));
    assert_eq!(
        check(&point, &bad).map_err(|f| f.field),
        Err("a.x".to_owned())
    );
}

#[test]
fn point_faults_name_the_missing_or_extra_field() {
    let point = arg(Kind::Point);
    let field = |v: Value| check(&point, &v).map_err(|f| f.field).err();
    assert_eq!(field(json!({"x": 5})), Some("a.y".to_owned()));
    assert_eq!(field(json!({"y": 5})), Some("a.x".to_owned()));
    assert_eq!(field(json!({"ref": "bad"})), Some("a.ref".to_owned()));
    assert_eq!(
        field(json!({"ref": "@s:e1", "x": 1, "y": 2})),
        Some("a".to_owned())
    );
    assert_eq!(field(json!({"z": 1})), Some("a.z".to_owned()));
}

#[test]
fn ref_then_text_splits_a_leading_ref_only_before_text() {
    let first = words(&["@s:e1", "hi", "there"]);
    let (reference, text) = split_ref_text(&first);
    assert_eq!((reference, text.as_str()), (Some("@s:e1"), "hi there"));
    let second = words(&["@notaref", "x"]);
    let (reference, text) = split_ref_text(&second);
    assert_eq!((reference, text.as_str()), (None, "@notaref x"));
}

#[test]
fn bounded_numbers_reject_both_edges_and_non_finite() {
    let duration = arg(Kind::Number {
        gt: Some(0.0),
        max: Some(10.0),
        default: Some(1.0),
        except: None,
    });
    for bad in ["0", "-1", "11", "nan", "inf", "long"] {
        let value = from_words(duration.kind, &words(&[bad]));
        assert!(check(&duration, &value).is_err(), "{bad}");
    }
    assert_eq!(check(&duration, &json!(10)), Ok(json!(10.0)));
}

#[test]
fn app_ids_and_enums_name_the_field_and_the_allowed_shape() {
    let app = arg(Kind::AppId);
    for bad in [json!("a;b"), json!(""), json!(3)] {
        let fault = check(&app, &bad).err();
        assert_eq!(fault.map(|f| f.field), Some("a".to_owned()), "{bad}");
    }
    let enum_arg = arg(Kind::Enum(&["up", "down"]));
    let fault = check(&enum_arg, &json!("sideways")).err();
    assert!(fault.is_some_and(|f| f.fix.contains("up, down")));
}

#[test]
fn unknown_keys_list_what_the_verb_accepts() {
    let input = json!({"token": "secret"});
    let fault = find("status")
        .zip(input.as_object())
        .and_then(|(status, m)| check_args(status, m, false).err());
    assert!(fault.is_some_and(|f| f.field == "token" && f.fix.contains("device")));
}

#[test]
fn paths_are_cli_only_keys() {
    let input = json!({"output": "/tmp/x.png"});
    let pair = find("screenshot").zip(input.as_object());
    assert!(pair.is_some_and(|(shot, m)| check_args(shot, m, false).is_err()));
    assert!(pair.is_some_and(|(shot, m)| check_args(shot, m, true).is_ok()));
}

#[test]
fn defaults_fill_omitted_numbers() {
    let input = json!({"target": {"ref": "@s:e1"}});
    let args = find("hold")
        .zip(input.as_object())
        .and_then(|(hold, m)| check_args(hold, m, false).ok());
    assert_eq!(
        args.and_then(|a| a.get("duration").cloned()),
        Some(json!(1.0))
    );
}

#[test]
fn pinch_scales_near_one_fail_locally() {
    let input = |scale: f64| json!({"ref": "@s:e1", "scale": scale});
    for (scale, ok) in [
        (1.0, false),
        (1.005, false),
        (0.995, false),
        (1.02, true),
        (0.5, true),
    ] {
        let value = input(scale);
        let result = find("pinch")
            .zip(value.as_object())
            .map(|(pinch, m)| check_args(pinch, m, false));
        assert_eq!(result.map(|r| r.is_ok()), Some(ok), "{scale}");
    }
}
