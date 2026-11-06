use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectDef {
    pub name: String,
    pub keywords: Vec<String>,
    pub enabled: bool,
}

pub fn general_defaults() -> Vec<SubjectDef> {
    vec![
        subject(
            "Mathematics",
            &[
                "fourier",
                "calculus",
                "algebra",
                "matrix",
                "matrices",
                "integral",
                "derivative",
                "geometry",
                "trigonometry",
                "statistics",
                "probability",
                "linear algebra",
                "differential",
                "math",
                "mathematics",
            ],
        ),
        subject(
            "Physics",
            &[
                "maxwell",
                "quantum",
                "relativity",
                "newton",
                "thermodynamics",
                "mechanics",
                "electromagnetism",
                "optics",
                "kinematics",
                "physics",
                "wave equation",
                "schrodinger",
                "photon",
                "gravity",
            ],
        ),
        subject(
            "Electronics",
            &[
                "circuit",
                "arduino",
                "pcb",
                "transistor",
                "resistor",
                "capacitor",
                "voltage",
                "current",
                "electronics",
                "microcontroller",
                "op-amp",
                "opamp",
                "schematic",
                "diode",
                "fpga",
                "verilog",
                "vhdl",
            ],
        ),
        subject(
            "Programming",
            &[
                "python",
                "javascript",
                "typescript",
                "algorithm",
                "programming",
                "software",
                "compiler",
                "database",
                "react",
                "rust",
                "debugging",
                "leetcode",
            ],
        ),
        subject(
            "Engineering",
            &[
                "cad",
                "mechanical",
                "thermal",
                "engineering",
                "structural",
                "civil",
                "finite element",
                "fea",
                "solidworks",
                "autocad",
                "manufacturing",
            ],
        ),
    ]
}

/// Starter set for B.Tech Electrical / EEE-style coursework.
pub fn btech_electrical_preset() -> Vec<SubjectDef> {
    vec![
        subject(
            "Circuit Theory",
            &[
                "circuit theory",
                "network theory",
                "kirchhoff",
                "thevenin",
                "norton",
                "mesh analysis",
                "nodal analysis",
                "rlc",
                "transient",
            ],
        ),
        subject(
            "Electrical Machines",
            &[
                "electrical machines",
                "induction motor",
                "synchronous machine",
                "dc machine",
                "transformer",
                "alternator",
                "motor",
                "machines lab",
            ],
        ),
        subject(
            "Power Systems",
            &[
                "power system",
                "power systems",
                "transmission",
                "distribution",
                "load flow",
                "fault analysis",
                "swing equation",
                "grid",
                "substation",
            ],
        ),
        subject(
            "Power Electronics",
            &[
                "power electronics",
                "inverter",
                "rectifier",
                "thyristor",
                "scr",
                "chopper",
                "converter",
                "smps",
                "igbt",
            ],
        ),
        subject(
            "Control Systems",
            &[
                "control system",
                "control systems",
                "pid",
                "bode",
                "nyquist",
                "root locus",
                "transfer function",
                "stability",
                "state space",
            ],
        ),
        subject(
            "Signals and Systems",
            &[
                "signals and systems",
                "signal processing",
                "fourier",
                "laplace",
                "z-transform",
                "convolution",
                "sampling",
                "dsp",
                "dft",
                "fft",
            ],
        ),
        subject(
            "Digital Electronics",
            &[
                "digital electronics",
                "boolean",
                "flip flop",
                "flip-flop",
                "logic gate",
                "multiplexer",
                "verilog",
                "vhdl",
                "fsm",
                "sequential circuit",
            ],
        ),
        subject(
            "Analog Electronics",
            &[
                "analog electronics",
                "op-amp",
                "opamp",
                "amplifier",
                "bjt",
                "mosfet",
                "diode",
                "biasing",
                "oscillator",
            ],
        ),
        subject(
            "Electromagnetics",
            &[
                "electromagnetics",
                "electromagnetic",
                "maxwell",
                "gauss",
                "faraday",
                "transmission line",
                "waveguide",
                "antenna",
                "emft",
            ],
        ),
        subject(
            "Microcontrollers",
            &[
                "microcontroller",
                "microprocessor",
                "8051",
                "arduino",
                "embedded",
                "arm",
                "stm32",
                "interfacing",
            ],
        ),
        subject(
            "Measurements",
            &[
                "measurements",
                "instrumentation",
                "wattmeter",
                "bridge",
                "oscilloscope",
                "transducer",
                "sensor",
            ],
        ),
        subject(
            "Mathematics",
            &[
                "calculus",
                "algebra",
                "differential",
                "laplace",
                "matrix",
                "probability",
                "engineering mathematics",
                "m1",
                "m2",
                "m3",
            ],
        ),
        subject(
            "Programming",
            &[
                "python",
                "c programming",
                "matlab",
                "simulink",
                "coding",
                "algorithm",
                "labview",
            ],
        ),
    ]
}

pub fn preset_list() -> Vec<(&'static str, &'static str)> {
    vec![
        ("general", "General study subjects"),
        ("btech-electrical", "B.Tech Electrical / EEE"),
    ]
}

pub fn preset_by_id(id: &str) -> Option<Vec<SubjectDef>> {
    match id {
        "general" => Some(general_defaults()),
        "btech-electrical" => Some(btech_electrical_preset()),
        _ => None,
    }
}

fn subject(name: &str, keywords: &[&str]) -> SubjectDef {
    SubjectDef {
        name: name.to_string(),
        keywords: keywords.iter().map(|k| (*k).to_string()).collect(),
        enabled: true,
    }
}
