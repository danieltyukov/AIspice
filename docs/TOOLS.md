# Tools

Generated with `aispice docs tools`. The desktop app's agent and the MCP server expose the same tools.

## `list_circuits`

List the circuit files in the open project (.asc schematics and SPICE netlists), newest first. Call this first when you do not know the file names.

<details><summary>Input schema</summary>

```json
{
  "properties": {},
  "type": "object"
}
```

</details>

## `read_schematic`

Describe a schematic in circuit terms: every component with its value and the net on each pin, every net with its members, the directives, and electrical rule check results. Read a circuit before editing it. Pin names shown here (A, B, +, -, C, B, E, D, G, S, In+, ...) are the ones edit_schematic expects.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "description": "Path of the circuit relative to the project folder, e.g. `rc.asc`.",
      "type": "string"
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `edit_schematic`

Change a schematic with circuit-level edits. Refer to parts by instance name (R1) and to pins as PART.PIN (R1.A, R1.2, Q1.B, V1.+, U1.In-). Never compute coordinates: `connect` routes wires itself and only ever joins the two nets you name (it falls back to net labels when no clean route exists), `connect_to_net` attaches a pin to a named net or to ground (`0`), and `add_component` without `at` finds free space near `near`. Typical sequence for a new part: add_component, then connect or connect_to_net for each pin. Two-terminal parts are vertical by default; orient R90 makes them horizontal. Directives (.tran, .ac, .op, .param, .meas, .step) go in with add_directive.
The edit is saved immediately (the user can undo it) and LTspice reloads if it is open. The result lists what changed, a semantic diff, and any electrical rule problems the edit introduced; fix those before simulating.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "description": "Path of the schematic relative to the project folder.",
      "type": "string"
    },
    "edits": {
      "description": "Edits, applied in order and atomically: if one fails, none are kept.",
      "items": {
        "description": "One edit. Required fields per op: add_component(symbol; optional name, value, orient, near, at, attrs) | remove(name) | replace_symbol(name, symbol) | move(name, to=[x,y]) | rotate(name; optional orient) | set_value(name, value) | set_attr(name, key, value) | rename(name, new_name) | connect(from=PIN, to=PIN) | connect_to_net(pin, net) | disconnect(pin) | add_wire(from=[x,y], to=[x,y]) | remove_wire(from=[x,y], to=[x,y]) | add_label(at, label) | remove_label(label; optional at) | add_directive(text; optional at) | remove_directive(matching) | replace_directive(matching, text) | add_comment(text; optional at). PIN is PART.PIN such as R1.A, R1.2, Q1.B, V1.+, U1.In-.",
        "properties": {
          "at": {
            "description": "add_component, add_label, remove_label, add_directive, add_comment: a sheet position [x, y]. Leave out for automatic placement.",
            "items": {
              "type": "integer"
            },
            "maxItems": 2,
            "minItems": 2,
            "type": "array"
          },
          "attrs": {
            "additionalProperties": {
              "type": "string"
            },
            "description": "add_component: extra attributes such as {\"SpiceLine\": \"AC 1\"}.",
            "type": "object"
          },
          "from": {
            "anyOf": [
              {
                "type": "string"
              },
              {
                "items": {
                  "type": "integer"
                },
                "maxItems": 2,
                "minItems": 2,
                "type": "array"
              }
            ],
            "description": "connect: a pin such as R1.B. add_wire, remove_wire: a point [x, y]."
          },
          "key": {
            "description": "set_attr: attribute name (Value, Value2, SpiceLine, SpiceLine2, SpiceModel, Prefix).",
            "type": "string"
          },
          "label": {
            "description": "add_label, remove_label: the net label text.",
            "type": "string"
          },
          "matching": {
            "description": "remove_directive, replace_directive: text that identifies the directive, e.g. .tran.",
            "type": "string"
          },
          "name": {
            "description": "The component's instance name (R1). For add_component, optional: defaults to the next free name.",
            "type": "string"
          },
          "near": {
            "description": "add_component: place next to this component.",
            "type": "string"
          },
          "net": {
            "description": "connect_to_net: the net name; 0 or gnd for ground.",
            "type": "string"
          },
          "new_name": {
            "description": "rename: the new instance name.",
            "type": "string"
          },
          "op": {
            "description": "The operation.",
            "enum": [
              "add_component",
              "remove",
              "replace_symbol",
              "move",
              "rotate",
              "set_value",
              "set_attr",
              "rename",
              "connect",
              "connect_to_net",
              "disconnect",
              "add_wire",
              "remove_wire",
              "add_label",
              "remove_label",
              "add_directive",
              "remove_directive",
              "replace_directive",
              "add_comment"
            ],
            "type": "string"
          },
          "orient": {
            "description": "add_component, rotate: R0 (default, vertical two-terminal parts), R90 (horizontal), R180, R270, M0, M90, M180, M270.",
            "enum": [
              "R0",
              "R90",
              "R180",
              "R270",
              "M0",
              "M90",
              "M180",
              "M270"
            ],
            "type": "string"
          },
          "pin": {
            "description": "connect_to_net, disconnect: a pin such as V1.- or C1.B.",
            "type": "string"
          },
          "symbol": {
            "description": "add_component, replace_symbol: symbol name such as res, cap, ind, voltage, current, diode, npn, pnp, nmos, pmos, OpAmps\\opamp, OpAmps\\opamp2, bv, e, g, sw, or a library part.",
            "type": "string"
          },
          "text": {
            "description": "add_directive, replace_directive, add_comment: the text, e.g. .tran 10m or .ac dec 20 10 100k.",
            "type": "string"
          },
          "to": {
            "anyOf": [
              {
                "type": "string"
              },
              {
                "items": {
                  "type": "integer"
                },
                "maxItems": 2,
                "minItems": 2,
                "type": "array"
              }
            ],
            "description": "connect: a pin such as C1.A. move: the new position [x, y]. add_wire, remove_wire: a point [x, y]."
          },
          "value": {
            "description": "add_component, set_value, set_attr: the value, e.g. 4.7k, 100n, SINE(0 1 1k).",
            "type": "string"
          }
        },
        "required": [
          "op"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "reason": {
      "description": "One short sentence saying why, shown in the history.",
      "type": "string"
    }
  },
  "required": [
    "circuit",
    "edits"
  ],
  "type": "object"
}
```

</details>

## `create_schematic`

Create a new LTspice schematic in the project. Give a SPICE netlist to have it drawn as a readable schematic (parts placed, wires routed, checked to netlist back to the same circuit), or build it from an empty sheet with the edits edit_schematic accepts, or both (edits apply after the drawing). Refuses to overwrite an existing file.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "description": "New file path relative to the project folder, ending in `.asc`.",
      "type": "string"
    },
    "edits": {
      "default": [],
      "description": "Edits to apply after the netlist is drawn, or to build the circuit\nfrom an empty sheet (same operations as edit_schematic).",
      "items": {
        "description": "One edit. Required fields per op: add_component(symbol; optional name, value, orient, near, at, attrs) | remove(name) | replace_symbol(name, symbol) | move(name, to=[x,y]) | rotate(name; optional orient) | set_value(name, value) | set_attr(name, key, value) | rename(name, new_name) | connect(from=PIN, to=PIN) | connect_to_net(pin, net) | disconnect(pin) | add_wire(from=[x,y], to=[x,y]) | remove_wire(from=[x,y], to=[x,y]) | add_label(at, label) | remove_label(label; optional at) | add_directive(text; optional at) | remove_directive(matching) | replace_directive(matching, text) | add_comment(text; optional at). PIN is PART.PIN such as R1.A, R1.2, Q1.B, V1.+, U1.In-.",
        "properties": {
          "at": {
            "description": "add_component, add_label, remove_label, add_directive, add_comment: a sheet position [x, y]. Leave out for automatic placement.",
            "items": {
              "type": "integer"
            },
            "maxItems": 2,
            "minItems": 2,
            "type": "array"
          },
          "attrs": {
            "additionalProperties": {
              "type": "string"
            },
            "description": "add_component: extra attributes such as {\"SpiceLine\": \"AC 1\"}.",
            "type": "object"
          },
          "from": {
            "anyOf": [
              {
                "type": "string"
              },
              {
                "items": {
                  "type": "integer"
                },
                "maxItems": 2,
                "minItems": 2,
                "type": "array"
              }
            ],
            "description": "connect: a pin such as R1.B. add_wire, remove_wire: a point [x, y]."
          },
          "key": {
            "description": "set_attr: attribute name (Value, Value2, SpiceLine, SpiceLine2, SpiceModel, Prefix).",
            "type": "string"
          },
          "label": {
            "description": "add_label, remove_label: the net label text.",
            "type": "string"
          },
          "matching": {
            "description": "remove_directive, replace_directive: text that identifies the directive, e.g. .tran.",
            "type": "string"
          },
          "name": {
            "description": "The component's instance name (R1). For add_component, optional: defaults to the next free name.",
            "type": "string"
          },
          "near": {
            "description": "add_component: place next to this component.",
            "type": "string"
          },
          "net": {
            "description": "connect_to_net: the net name; 0 or gnd for ground.",
            "type": "string"
          },
          "new_name": {
            "description": "rename: the new instance name.",
            "type": "string"
          },
          "op": {
            "description": "The operation.",
            "enum": [
              "add_component",
              "remove",
              "replace_symbol",
              "move",
              "rotate",
              "set_value",
              "set_attr",
              "rename",
              "connect",
              "connect_to_net",
              "disconnect",
              "add_wire",
              "remove_wire",
              "add_label",
              "remove_label",
              "add_directive",
              "remove_directive",
              "replace_directive",
              "add_comment"
            ],
            "type": "string"
          },
          "orient": {
            "description": "add_component, rotate: R0 (default, vertical two-terminal parts), R90 (horizontal), R180, R270, M0, M90, M180, M270.",
            "enum": [
              "R0",
              "R90",
              "R180",
              "R270",
              "M0",
              "M90",
              "M180",
              "M270"
            ],
            "type": "string"
          },
          "pin": {
            "description": "connect_to_net, disconnect: a pin such as V1.- or C1.B.",
            "type": "string"
          },
          "symbol": {
            "description": "add_component, replace_symbol: symbol name such as res, cap, ind, voltage, current, diode, npn, pnp, nmos, pmos, OpAmps\\opamp, OpAmps\\opamp2, bv, e, g, sw, or a library part.",
            "type": "string"
          },
          "text": {
            "description": "add_directive, replace_directive, add_comment: the text, e.g. .tran 10m or .ac dec 20 10 100k.",
            "type": "string"
          },
          "to": {
            "anyOf": [
              {
                "type": "string"
              },
              {
                "items": {
                  "type": "integer"
                },
                "maxItems": 2,
                "minItems": 2,
                "type": "array"
              }
            ],
            "description": "connect: a pin such as C1.A. move: the new position [x, y]. add_wire, remove_wire: a point [x, y]."
          },
          "value": {
            "description": "add_component, set_value, set_attr: the value, e.g. 4.7k, 100n, SINE(0 1 1k).",
            "type": "string"
          }
        },
        "required": [
          "op"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "netlist": {
      "description": "A SPICE netlist to draw as a readable schematic: elements, `.model`\nand `.subckt` definitions and analysis directives, one per line. Use\nit to turn a netlist, a textbook circuit or a circuit read from an\nimage into a schematic. aispice places and wires every part and\nchecks that the drawing netlists back to exactly this circuit.",
      "type": "string"
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `lint`

Run electrical rule checks on a schematic: missing ground, floating pins, dangling wires, duplicate names, missing values, shorted parts, parallel voltage sources, nets with no DC path to ground, missing analysis directive. Run it after edits and before simulating.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "description": "Path of the circuit relative to the project folder, e.g. `rc.asc`.",
      "type": "string"
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `netlist`

Show the SPICE netlist of a schematic, derived the way LTspice does it (instance names, node order, default models). Useful to check exactly what will be simulated.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "description": "Path of the circuit relative to the project folder, e.g. `rc.asc`.",
      "type": "string"
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `history`

List a schematic's saved versions, or undo, redo or restore one. Every edit made by aispice and every change made in LTspice in between is a version.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "action": {
      "enum": [
        "list",
        "undo",
        "redo",
        "restore"
      ],
      "type": "string"
    },
    "circuit": {
      "type": "string"
    },
    "snapshot": {
      "description": "Snapshot id for `restore`, from `list`.",
      "type": "string"
    }
  },
  "required": [
    "circuit",
    "action"
  ],
  "type": "object"
}
```

</details>

## `render_schematic`

Draw a schematic as an image (PNG by default) so you can look at its layout: overlapping parts, crossing wires, unreadable labels. For connectivity, read_schematic is more precise than looking.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "format": {
      "anyOf": [
        {
          "description": "A PNG image, for models that read images.",
          "enum": [
            "png"
          ],
          "type": "string"
        },
        {
          "description": "SVG markup as text.",
          "enum": [
            "svg"
          ],
          "type": "string"
        }
      ]
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `symbols`

Search the symbols available for add_component: aispice's built-in set and, when installed, LTspice's library (thousands of vendor parts such as LT1001 or ADA4898). Shows each symbol's name, prefix and pin names in SPICE order.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "limit": {
      "default": 25,
      "minimum": 0,
      "type": "integer"
    },
    "query": {
      "default": "",
      "description": "Words to match in symbol names and descriptions, e.g. `opamp`,\n`schottky`, `LT1001`. Empty lists the common ones.",
      "type": "string"
    }
  },
  "type": "object"
}
```

</details>

## `simulators`

List the simulators installed on this machine (ngspice, LTspice, Xyce, Spectre) with versions and how each will run.

<details><summary>Input schema</summary>

```json
{
  "properties": {},
  "type": "object"
}
```

</details>

## `simulate`

Simulate a circuit and report what came back: analyses, vector names, operating point, the simulator's own .meas results, warnings and errors, plus any measurements you ask for. Fix lint errors first. Use the run id with measure, plot and read_waveform. Measurements are written `name = kind(args)`, for example `f3db = bandwidth_3db(V(out))`, `gain = gain_db_at(V(out)/V(in), 1k)`, `pm = phase_margin(V(out))`, `tr = rise_time(V(out), 10, 90)`, `vmax = max(V(out), 1m, 5m)`. Kinds: value_at(expr, at), min/max/pp/avg/rms/integral(expr[, from, to]), crossing(expr, level[, rise|fall|either, nth]), rise_time/fall_time(expr[, low_pct, high_pct]), overshoot_pct/undershoot_pct(expr), settling_time(expr[, tolerance_pct]), delay(from_expr, to_expr[, level_pct]), frequency/period/duty_cycle(expr), thd(expr, fundamental[, harmonics]), gain_db_at/phase_at(expr, freq), bandwidth_3db(expr[, dc|peak]), unity_gain_freq/phase_margin/gain_margin/peak_gain(expr), freq_at_db(expr, db). Expressions use V(node), V(a,b), I(R1), + - * /, and db(), mag(), ph().

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "analysis": {
      "description": "Run this analysis instead of the circuit's own, without changing the\nfile, e.g. `.ac dec 50 1 10Meg` or `.tran 0 5m 0 1u`.",
      "type": "string"
    },
    "circuit": {
      "type": "string"
    },
    "measurements": {
      "default": [],
      "description": "Measurements to take on the result (see the tool description).",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "simulator": {
      "description": "`ngspice`, `ltspice`, `xyce` or `spectre`. Default: automatic\n(ngspice, falling back to LTspice for LTspice-only constructs).",
      "type": "string"
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `measure`

Take measurements on a simulation result without re-running it. Measurements are written `name = kind(args)`, for example `f3db = bandwidth_3db(V(out))`, `gain = gain_db_at(V(out)/V(in), 1k)`, `pm = phase_margin(V(out))`, `tr = rise_time(V(out), 10, 90)`, `vmax = max(V(out), 1m, 5m)`. Kinds: value_at(expr, at), min/max/pp/avg/rms/integral(expr[, from, to]), crossing(expr, level[, rise|fall|either, nth]), rise_time/fall_time(expr[, low_pct, high_pct]), overshoot_pct/undershoot_pct(expr), settling_time(expr[, tolerance_pct]), delay(from_expr, to_expr[, level_pct]), frequency/period/duty_cycle(expr), thd(expr, fundamental[, harmonics]), gain_db_at/phase_at(expr, freq), bandwidth_3db(expr[, dc|peak]), unity_gain_freq/phase_margin/gain_margin/peak_gain(expr), freq_at_db(expr, db). Expressions use V(node), V(a,b), I(R1), + - * /, and db(), mag(), ph().

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "measurements": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "run": {
      "description": "A run id from simulate. Default: the circuit's latest run, simulating\nfirst if there is none.",
      "type": "string"
    }
  },
  "required": [
    "circuit",
    "measurements"
  ],
  "type": "object"
}
```

</details>

## `check_specs`

Simulate the circuit and check it against a spec table, reporting pass or fail and the margin for each spec. This is how to prove a design meets its requirements; run it after every change that matters. Specs are one per line: `name = measurement op limit`, with op one of `>=`, `<=`, `in a..b` or `= target +- tol`, e.g. `bw = bandwidth_3db(V(out)) in 1Meg..2Meg`, `gain = gain_db_at(V(out)/V(in), 1k) >= 20`, `pm = phase_margin(V(out)) >= 45`. Measurements are written `name = kind(args)`, for example `f3db = bandwidth_3db(V(out))`, `gain = gain_db_at(V(out)/V(in), 1k)`, `pm = phase_margin(V(out))`, `tr = rise_time(V(out), 10, 90)`, `vmax = max(V(out), 1m, 5m)`. Kinds: value_at(expr, at), min/max/pp/avg/rms/integral(expr[, from, to]), crossing(expr, level[, rise|fall|either, nth]), rise_time/fall_time(expr[, low_pct, high_pct]), overshoot_pct/undershoot_pct(expr), settling_time(expr[, tolerance_pct]), delay(from_expr, to_expr[, level_pct]), frequency/period/duty_cycle(expr), thd(expr, fundamental[, harmonics]), gain_db_at/phase_at(expr, freq), bandwidth_3db(expr[, dc|peak]), unity_gain_freq/phase_margin/gain_margin/peak_gain(expr), freq_at_db(expr, db). Expressions use V(node), V(a,b), I(R1), + - * /, and db(), mag(), ph().

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "save": {
      "default": true,
      "description": "Save the given specs to `<circuit>.specs` for later checks. Default true.",
      "type": "boolean"
    },
    "simulator": {
      "type": "string"
    },
    "specs": {
      "description": "Specs, one per line. When omitted, the `<circuit>.specs` file beside\nthe circuit is used.",
      "type": "string"
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `plot`

Plot waveforms from a simulation as an image: time-domain traces, or a Bode plot (magnitude in dB and phase) for AC data. Use it to look at behaviour; use measure for numbers.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "run": {
      "type": "string"
    },
    "title": {
      "type": "string"
    },
    "traces": {
      "description": "Expressions to draw, e.g. `V(out)`, `V(out)/V(in)` (a Bode plot for AC\ndata), `I(R1)`.",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "x_range": {
      "description": "Limit the x axis, e.g. `[0, 0.001]`.",
      "items": {
        "type": "number"
      },
      "maxItems": 2,
      "minItems": 2,
      "type": "array"
    }
  },
  "required": [
    "circuit",
    "traces"
  ],
  "type": "object"
}
```

</details>

## `read_waveform`

Read sampled values of signals from a simulation as a table (evenly spaced over a range). For AC data values are magnitude in dB and phase in degrees.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "from": {
      "type": "number"
    },
    "points": {
      "description": "Number of evenly spaced rows to return (default 40, at most 400).",
      "minimum": 0,
      "type": "integer"
    },
    "run": {
      "type": "string"
    },
    "signals": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "to": {
      "type": "number"
    }
  },
  "required": [
    "circuit",
    "signals"
  ],
  "type": "object"
}
```

</details>

## `poles_zeros`

Pole-zero analysis of a transfer function with ngspice's .pz: the poles and zeros of V(output)/V(input), or V(output)/I(input) with transfer `cur`, in Hz, with f0, Q and damping ratio for each complex pair and the corner of each real root, and whether the circuit is stable (any pole in the right half-plane means it is not). Nonlinear parts are linearised at the operating point, as in an AC analysis. The circuit's own analyses, .meas, .step and .save lines are left out of this run, and the file is not changed. Needs ngspice: LTspice and Xyce have no .pz analysis.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "input": {
      "description": "Positive input node (a net name as read_schematic shows it).",
      "type": "string"
    },
    "input_neg": {
      "description": "Negative input node. Default: ground (`0`).",
      "type": "string"
    },
    "output": {
      "description": "Positive output node.",
      "type": "string"
    },
    "output_neg": {
      "description": "Negative output node. Default: ground (`0`).",
      "type": "string"
    },
    "params": {
      "additionalProperties": {
        "anyOf": [
          {
            "type": "number"
          },
          {
            "type": "string"
          }
        ],
        "description": "A number as JSON or as SPICE text (`2.2k`)."
      },
      "description": "Values for this analysis only, without changing the file: an element\n(`R1`) or `.param` name and its value, e.g. `{\"R1\": \"2.2k\"}`.",
      "type": "object"
    },
    "transfer": {
      "anyOf": [
        {
          "description": "Voltage gain: V(output) / V(input).",
          "enum": [
            "vol"
          ],
          "type": "string"
        },
        {
          "description": "Transimpedance: V(output) / I(input), for a current driven into the\ninput pair.",
          "enum": [
            "cur"
          ],
          "type": "string"
        }
      ],
      "description": "`vol` for voltage gain (default) or `cur` for transimpedance."
    }
  },
  "required": [
    "circuit",
    "input",
    "output"
  ],
  "type": "object"
}
```

</details>

## `operating_point`

Check bias: the DC operating point with each semiconductor device's small-signal values, for sizing transistors (the gm/Id method). MOSFETs: region (cutoff, subthreshold, triode, saturation), id, vgs, vds, vbs, vth (von for level 1 to 3), vdsat, gm, gds, gmbs, gm/id, intrinsic gain gm/gds, cgs and cgd when the model has them, W and L. BJTs: region (active, saturation, cutoff, reverse active), ic, ib, beta, vbe, vce, gm, rpi, ro. Diodes: id, vd, rd. Also node voltages and source currents. Devices inside subcircuits are named by path, as X1.M7. Points out devices in triode or cutoff that look like they should be saturated, such as a current-mirror output. Values are in each device's own polarity (for a PMOS, vgs is vsg). The circuit's own analyses, .meas, .step and .save lines are left out of this run and the file is not changed. Device values need ngspice; without it only node voltages and source currents come back, from LTspice or Xyce.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "devices": {
      "default": [],
      "description": "Report only these devices: instance names such as `M1`, `X1.M7` for\nM7 inside subcircuit instance X1, or `X1` for every device in it.\nDefault: every MOSFET, BJT and diode.",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "params": {
      "additionalProperties": {
        "anyOf": [
          {
            "type": "number"
          },
          {
            "type": "string"
          }
        ],
        "description": "A number as JSON or as SPICE text (`2.2k`)."
      },
      "description": "Values for this analysis only, without changing the file: an element\n(`R1`) or `.param` name and its value, e.g. `{\"Rbias\": \"47k\"}`.",
      "type": "object"
    }
  },
  "required": [
    "circuit"
  ],
  "type": "object"
}
```

</details>

## `sweep`

Sweep component values or .param values (several parameters give every combination, at most 200 runs) and tabulate measurements for each. Works with every simulator. Measurements are written `name = kind(args)`, for example `f3db = bandwidth_3db(V(out))`, `gain = gain_db_at(V(out)/V(in), 1k)`, `pm = phase_margin(V(out))`, `tr = rise_time(V(out), 10, 90)`, `vmax = max(V(out), 1m, 5m)`. Kinds: value_at(expr, at), min/max/pp/avg/rms/integral(expr[, from, to]), crossing(expr, level[, rise|fall|either, nth]), rise_time/fall_time(expr[, low_pct, high_pct]), overshoot_pct/undershoot_pct(expr), settling_time(expr[, tolerance_pct]), delay(from_expr, to_expr[, level_pct]), frequency/period/duty_cycle(expr), thd(expr, fundamental[, harmonics]), gain_db_at/phase_at(expr, freq), bandwidth_3db(expr[, dc|peak]), unity_gain_freq/phase_margin/gain_margin/peak_gain(expr), freq_at_db(expr, db). Expressions use V(node), V(a,b), I(R1), + - * /, and db(), mag(), ph().

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "analysis": {
      "type": "string"
    },
    "circuit": {
      "type": "string"
    },
    "measurements": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "params": {
      "description": "Parameters to sweep: an element (`R1`) or a `.param` name, with values\nas a list `[1000, 2200, 4700]`, a range `{\"kind\": \"dec\", \"start\": 1000,\n\"stop\": 100000, \"points_per_decade\": 5}`, or text `\"lin 1k 10k 10\"`.",
      "items": {
        "properties": {
          "target": {
            "description": "An element (`R1`, `C2`, `V1`) or a `.param` name.",
            "type": "string"
          },
          "values": {
            "anyOf": [
              {
                "items": {
                  "type": "number"
                },
                "type": "array"
              },
              {
                "anyOf": [
                  {
                    "description": "`points` values evenly spaced from `start` to `stop`, both included.",
                    "properties": {
                      "kind": {
                        "enum": [
                          "lin"
                        ],
                        "type": "string"
                      },
                      "points": {
                        "minimum": 0,
                        "type": "integer"
                      },
                      "start": {
                        "type": "number"
                      },
                      "stop": {
                        "type": "number"
                      }
                    },
                    "required": [
                      "kind",
                      "start",
                      "stop",
                      "points"
                    ],
                    "type": "object"
                  },
                  {
                    "description": "Logarithmic, `points_per_decade` values per decade from `start`.",
                    "properties": {
                      "kind": {
                        "enum": [
                          "dec"
                        ],
                        "type": "string"
                      },
                      "points_per_decade": {
                        "minimum": 0,
                        "type": "integer"
                      },
                      "start": {
                        "type": "number"
                      },
                      "stop": {
                        "type": "number"
                      }
                    },
                    "required": [
                      "kind",
                      "start",
                      "stop",
                      "points_per_decade"
                    ],
                    "type": "object"
                  },
                  {
                    "description": "Logarithmic, `points_per_octave` values per octave from `start`.",
                    "properties": {
                      "kind": {
                        "enum": [
                          "oct"
                        ],
                        "type": "string"
                      },
                      "points_per_octave": {
                        "minimum": 0,
                        "type": "integer"
                      },
                      "start": {
                        "type": "number"
                      },
                      "stop": {
                        "type": "number"
                      }
                    },
                    "required": [
                      "kind",
                      "start",
                      "stop",
                      "points_per_octave"
                    ],
                    "type": "object"
                  }
                ]
              },
              {
                "type": "string"
              }
            ],
            "description": "The values to sweep: an explicit list, a range, or the text form\n(`\"1k, 2.2k, 4.7k\"`, `\"lin 1k 10k 10\"`, `\"dec 1k 1Meg 10\"`,\n`\"oct 1k 8k 2\"`; ranges are start, stop, count as in `.step`)."
          }
        },
        "required": [
          "target",
          "values"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "simulator": {
      "type": "string"
    }
  },
  "required": [
    "circuit",
    "params",
    "measurements"
  ],
  "type": "object"
}
```

</details>

## `monte_carlo`

Monte Carlo tolerance analysis: vary component values within their tolerances, simulate each sample, and report the yield against the specs with per-spec spread and the worst run. Seeded and reproducible. Specs are one per line: `name = measurement op limit`, with op one of `>=`, `<=`, `in a..b` or `= target +- tol`, e.g. `bw = bandwidth_3db(V(out)) in 1Meg..2Meg`, `gain = gain_db_at(V(out)/V(in), 1k) >= 20`, `pm = phase_margin(V(out)) >= 45`.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "circuit": {
      "type": "string"
    },
    "runs": {
      "description": "Number of runs (default 50, at most 500).",
      "minimum": 0,
      "type": "integer"
    },
    "seed": {
      "minimum": 0,
      "type": "integer"
    },
    "simulator": {
      "type": "string"
    },
    "specs": {
      "description": "Specs to judge each run; default: the `<circuit>.specs` file.",
      "type": "string"
    },
    "tolerances": {
      "description": "Tolerances: target is an element name or a glob such as `R*` or `C*`,\n`tol_pct` a percentage, `distribution` uniform (default) or gaussian\n(3 sigma = tolerance).",
      "items": {
        "properties": {
          "distribution": {
            "anyOf": [
              {
                "description": "Equally likely anywhere within plus or minus the tolerance.",
                "enum": [
                  "uniform"
                ],
                "type": "string"
              },
              {
                "description": "Normal with three standard deviations equal to the tolerance, as\ncomponent tolerances are usually specified.",
                "enum": [
                  "gaussian"
                ],
                "type": "string"
              }
            ],
            "default": "uniform"
          },
          "target": {
            "description": "An element or `.param` name, or a glob over element names such as\n`R*`, `C?` or `*`. Exact names take precedence over globs, and\nearlier globs over later ones.",
            "type": "string"
          },
          "tol_pct": {
            "description": "Tolerance in percent of the nominal value.",
            "type": "number"
          }
        },
        "required": [
          "target",
          "tol_pct"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "circuit",
    "tolerances"
  ],
  "type": "object"
}
```

</details>

## `optimize`

Size component values to meet a spec table with the simulator in the loop (Nelder-Mead or CMA-ES over log-scaled ranges), then round to standard E-series values and re-check. You choose the parameters and sensible ranges; the optimizer does the search. Specs are one per line: `name = measurement op limit`, with op one of `>=`, `<=`, `in a..b` or `= target +- tol`, e.g. `bw = bandwidth_3db(V(out)) in 1Meg..2Meg`, `gain = gain_db_at(V(out)/V(in), 1k) >= 20`, `pm = phase_margin(V(out)) >= 45`.

<details><summary>Input schema</summary>

```json
{
  "properties": {
    "apply": {
      "default": false,
      "description": "Write the best (snapped) values into the schematic as an undoable edit.",
      "type": "boolean"
    },
    "circuit": {
      "type": "string"
    },
    "goal": {
      "description": "Optionally also minimise or maximise one spec's value once all pass.",
      "properties": {
        "sense": {
          "enum": [
            "minimize",
            "maximize"
          ],
          "type": "string"
        },
        "spec": {
          "description": "Name of the spec row whose value to push.",
          "type": "string"
        },
        "weight": {
          "default": 0.01,
          "description": "Weight against the spec violations, which are squared fractions of\ntheir limits. Small weights keep the goal from trading a spec away.",
          "type": "number"
        }
      },
      "required": [
        "spec",
        "sense"
      ],
      "type": "object"
    },
    "max_evals": {
      "description": "Simulation budget (default 80, at most 400).",
      "minimum": 0,
      "type": "integer"
    },
    "method": {
      "anyOf": [
        {
          "description": "Fast and reliable for a few parameters.",
          "enum": [
            "nelder_mead"
          ],
          "type": "string"
        },
        {
          "description": "Better for many parameters or rough objectives.",
          "enum": [
            "cma_es"
          ],
          "type": "string"
        }
      ]
    },
    "params": {
      "description": "Parameters to size: an element (`R1`) or `.param` name with `min` and\n`max` (`\"1k\"` style values accepted), optional `log_scale` (default true\nfor R, C, L) and `snap` to an E-series (E6, E12, E24, E48, E96) for the\nfinal answer.",
      "items": {
        "description": "A value to size.",
        "properties": {
          "initial": {
            "description": "Starting value; the middle of the range (in the search space) if\nabsent.",
            "type": "number"
          },
          "log_scale": {
            "description": "Search in log space. Defaults to yes for names starting with R, C or\nL, whose sensible ranges span decades.",
            "type": "boolean"
          },
          "max": {
            "type": "number"
          },
          "min": {
            "type": "number"
          },
          "name": {
            "description": "An element or `.param` name, as in a sweep.",
            "type": "string"
          },
          "snap": {
            "description": "Round the final answer to this series.",
            "enum": [
              "E6",
              "E12",
              "E24",
              "E48",
              "E96",
              null
            ],
            "type": "string"
          }
        },
        "required": [
          "name",
          "min",
          "max"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "save_specs": {
      "default": true,
      "description": "Save the given specs to `<circuit>.specs`, as the circuit's\nrequirements from now on. Default true.",
      "type": "boolean"
    },
    "simulator": {
      "type": "string"
    },
    "specs": {
      "description": "Specs to meet; default: the `<circuit>.specs` file.",
      "type": "string"
    }
  },
  "required": [
    "circuit",
    "params"
  ],
  "type": "object"
}
```

</details>

