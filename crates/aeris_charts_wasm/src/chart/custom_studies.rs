//! Synchronous JS runtime adapter for engine-owned custom study bindings.

use super::*;
use aeris_charts_engine::{
    CustomStudyDefinition, CustomStudyFault, CustomStudyInput, CustomStudyOutput, CustomStudyPane,
    CustomStudyParams, CustomStudyPlot, CustomStudyRuntime, IndicatorInputSource,
    IndicatorOutputStyle, IndicatorParameterDescriptor,
};
use js_sys::{Array, Function, Object, Reflect};
use serde::Deserialize;

#[derive(Deserialize)]
struct Definition {
    #[serde(rename = "type")]
    type_id: String,
    version: u32,
    title: String,
    parameters: Vec<IndicatorParameterDescriptor>,
    outputs: Vec<Output>,
    #[serde(default)]
    uses_volume: bool,
}

#[derive(Deserialize)]
struct Output {
    name: String,
    plot: Plot,
    pane: Pane,
    #[serde(default)]
    default_style: IndicatorOutputStyle,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Plot {
    Line,
    Histogram,
    Area,
    Marker,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Pane {
    Price,
    Dedicated,
}

fn result_error(code: &str, message: &str) -> String {
    serde_json::json!({"ok": false, "error": {"code": code, "message": message}}).to_string()
}

fn js_fault(value: JsValue) -> CustomStudyFault {
    let message = Reflect::get(&value, &"message".into())
        .ok()
        .and_then(|message| message.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "custom study callback threw".into());
    CustomStudyFault { message }
}

fn field(object: &Object, name: &str, value: &JsValue) -> Result<(), CustomStudyFault> {
    Reflect::set(object, &name.into(), value)
        .map(|_| ())
        .map_err(js_fault)
}

/// The six JS input columns a callback reads. ponytail: they grow to the next power of two of
/// the row count and never shrink (about 6 x 8 MiB per binding at 1M rows, bounded by 32
/// bindings per chart); shrink on demand only if a measurement shows memory pressure.
struct Mirrors {
    columns: [Float64Array; 6],
    capacity: u32,
}

impl Mirrors {
    fn new() -> Self {
        Self {
            columns: std::array::from_fn(|_| Float64Array::new_with_length(0)),
            capacity: 0,
        }
    }

    fn fill(&mut self, input: &CustomStudyInput<'_>) {
        let length = input.times.len() as u32;
        if length > self.capacity {
            let capacity = length.next_power_of_two();
            for column in &mut self.columns {
                let next = Float64Array::new_with_length(capacity);
                next.set(column, 0);
                *column = next;
            }
            self.capacity = capacity;
            // When capacity grows the old prefix was copied; only the dirty suffix changes.
        }
        for row in input.from..input.times.len() {
            self.columns[0].set_index(row as u32, input.times[row] as f64);
            self.columns[1].set_index(row as u32, input.open[row]);
            self.columns[2].set_index(row as u32, input.high[row]);
            self.columns[3].set_index(row as u32, input.low[row]);
            self.columns[4].set_index(row as u32, input.close[row]);
            self.columns[5].set_index(
                row as u32,
                input.volume.get(row).copied().unwrap_or(f64::NAN),
            );
        }
    }
}

struct JsCustomStudy {
    state: JsValue,
    update: Option<Function>,
    rebuild: Function,
    mirrors: Mirrors,
}

impl CustomStudyRuntime for JsCustomStudy {
    fn compute(
        &mut self,
        input: CustomStudyInput<'_>,
        out: &mut [Vec<f64>],
    ) -> Result<(), CustomStudyFault> {
        self.mirrors.fill(&input);
        let length = input.times.len() as u32;
        let ctx = Object::new();
        field(&ctx, "from", &JsValue::from_f64(input.from as f64))?;
        field(&ctx, "length", &JsValue::from_f64(length as f64))?;
        field(&ctx, "tail", &JsValue::from_bool(input.tail))?;
        for (index, name) in ["time", "open", "high", "low", "close", "volume"]
            .iter()
            .enumerate()
        {
            field(
                &ctx,
                name,
                &self.mirrors.columns[index].subarray(0, length).into(),
            )?;
        }
        let outputs = Array::new();
        for _ in 0..out.len() {
            let values = Float64Array::new_with_length(length - input.from as u32);
            values.fill(f64::NAN, 0, values.length());
            outputs.push(&values);
        }
        field(&ctx, "outputs", outputs.as_ref())?;
        let callback = if input.tail {
            self.update.as_ref().unwrap_or(&self.rebuild)
        } else {
            &self.rebuild
        };
        callback
            .call2(&JsValue::UNDEFINED, &self.state, &ctx)
            .map_err(js_fault)?;
        for (index, values) in out.iter_mut().enumerate() {
            values.extend(Float64Array::new(&outputs.get(index as u32)).to_vec());
        }
        Ok(())
    }
}

pub(super) fn register_custom_study(
    engine: &mut ChartEngine,
    json: &str,
    init: Function,
    update: Option<Function>,
    rebuild: Function,
) -> String {
    let definition: Definition = match serde_json::from_str(json) {
        Ok(definition) => definition,
        Err(error) => return result_error("invalid_options", &error.to_string()),
    };
    let def = CustomStudyDefinition {
        type_id: definition.type_id,
        version: definition.version,
        title: definition.title,
        parameters: definition.parameters,
        outputs: definition
            .outputs
            .into_iter()
            .map(|output| CustomStudyOutput {
                name: output.name,
                plot: match output.plot {
                    Plot::Line => CustomStudyPlot::Line,
                    Plot::Histogram => CustomStudyPlot::Histogram,
                    Plot::Area => CustomStudyPlot::Area,
                    Plot::Marker => CustomStudyPlot::Marker,
                },
                pane: match output.pane {
                    Pane::Price => CustomStudyPane::Price,
                    Pane::Dedicated => CustomStudyPane::Dedicated,
                },
                default_style: output.default_style,
            })
            .collect(),
        uses_volume: definition.uses_volume,
    };
    let factory = Box::new(move |params: &CustomStudyParams| {
        let params =
            js_sys::JSON::parse(&serde_json::to_string(params).unwrap()).map_err(js_fault)?;
        let state = init.call1(&JsValue::UNDEFINED, &params).map_err(js_fault)?;
        Ok(Box::new(JsCustomStudy {
            state,
            update: update.clone(),
            rebuild: rebuild.clone(),
            mirrors: Mirrors::new(),
        }) as Box<dyn CustomStudyRuntime>)
    });
    match engine.register_custom_study(def, factory) {
        Ok(()) => r#"{"ok":true}"#.into(),
        Err(error) => result_error(error.code().name(), error.message()),
    }
}

impl ChartInner {
    pub fn add_custom_study_result_json(
        &mut self,
        type_id: &str,
        source: u32,
        input: &str,
        volume: i64,
        parameters: &str,
    ) -> String {
        let Ok(source_input) =
            serde_json::from_value::<IndicatorInputSource>(serde_json::json!(input))
        else {
            return result_error("invalid_options", "invalid study input source");
        };
        let Ok(params) = serde_json::from_str::<CustomStudyParams>(parameters) else {
            return result_error("invalid_options", "invalid custom study parameters");
        };
        match self.engine.add_custom_study(
            type_id,
            source,
            source_input,
            (volume >= 0).then_some(volume as u32),
            params,
        ) {
            Ok(outputs) => serde_json::json!({"ok": true, "outputs": outputs}).to_string(),
            Err(error) => result_error(error.code().name(), error.message()),
        }
    }
}
