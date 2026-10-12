//! SREP 85: the rules read an `xs:double` or `xs:decimal` attribute (and the integers of SREP 67's ITR1) by its
//! value, whatever its sign: "+1" and "+0.5" are 1 and 0.5 (XSD 1.0 Part 2, 3.2.3.1 and 3.2.5.1), as the model reads
//! them (`parse_xsd_double`), where XPath 1.0's `number()` reads NaN. The documents are sr-core's kit cases
//! srep-0085-* (`conformance/srep_cases/srep-0085.json` of sr-core 1.6.0, byte-identical, with no pending marker),
//! copied beside this file: C13, C14, C15, C37, C38, C44, C52, C53, C54, C56, PYRO1, PYRO2, PYRO4, VOL4, P3D3, P3D10,
//! OCN3, OCN5, SREP 78's PYRO11 and PYC6, SREP 80's FRX11, and SREP 67's ITR1 and SRT1. One case writes an exponent
//! (`speed="1e0"`), for which the native validator is normative (SREP 85, Semantics 4).

use std::collections::BTreeSet;

fn errors(xml: &str) -> BTreeSet<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.code)
        .collect()
}

fn doc(version: &str, body: &str) -> String {
    format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="2"/><composition>{body}</composition></scene>"#
    )
}

#[test]
fn the_kit_cases_of_srep_85_have_their_verdicts() {
    let kit: serde_json::Value = serde_json::from_str(include_str!("srep-0085.json")).expect("the kit file is JSON");
    let cases = kit.as_object().expect("an object of cases");
    assert_eq!(cases.len(), 65);
    let mut wrong = Vec::new();
    for (name, case) in cases {
        let findings = &case["expected"]["findings"];
        let want: BTreeSet<String> = if findings["valid"].as_bool() == Some(true) {
            BTreeSet::new()
        } else {
            findings["codes"]
                .as_array()
                .expect("codes")
                .iter()
                .map(|c| c.as_str().expect("a code").to_string())
                .collect()
        };
        let got = errors(case["xml"].as_str().expect("the document"));
        if got != want {
            wrong.push(format!("{name}: want {want:?}, got {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn ocean_reads_the_foam_radius_by_value() {
    // OCN14 (SREP 79) already reads the sign in the Schematron, with translate(@foamRadius,'+',''): the native
    // mirror read "+1" as NaN and refused it
    let ocean = |foam: &str| {
        doc(
            "1.3",
            &format!(
                r#"<ocean id="sea" width="8" depth="8" cellSize="0.5"><whitewater emissionRate="2" threshold="0.3" foamRadius="{foam}"/></ocean>"#
            ),
        )
    };
    for foam in ["1", "+1", "+0.5", " +32 "] {
        assert_eq!(errors(&ocean(foam)), BTreeSet::new(), "{foam}");
    }
    for foam in ["33", "+33"] {
        assert_eq!(errors(&ocean(foam)), BTreeSet::from(["OCN14".to_string()]), "{foam}");
    }
}
