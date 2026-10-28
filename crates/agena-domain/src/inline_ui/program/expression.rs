use super::*;

// Evaluation steps alone do not bound allocation: a small map/DAG can duplicate
// large values. Charge the resolved tree before exposing or caching each value.
const MAX_RESULT_BYTES: usize = 256 * 1024;
const MAX_PRODUCED_BYTES: usize = 4 * 1024 * 1024;
fn result_size(value: &Value, bytes: &mut usize, depth: usize) -> Result<(), String> {
    *bytes = bytes.saturating_add(16);
    if depth > 64 || *bytes > MAX_RESULT_BYTES {
        return Err("UI result budget exceeded".into());
    }
    match value {
        Value::String(text) => *bytes = bytes.saturating_add(text.len()),
        Value::Array(items) => {
            for item in items {
                result_size(item, bytes, depth + 1)?;
            }
        }
        Value::Object(items) => {
            for (key, value) in items {
                *bytes = bytes.saturating_add(key.len());
                result_size(value, bytes, depth + 1)?;
            }
        }
        _ => {}
    }
    if *bytes > MAX_RESULT_BYTES {
        return Err("UI result budget exceeded".into());
    }
    Ok(())
}

const OPERATORS: &[&str] = &[
    "add", "sub", "mul", "div", "min", "max", "round", "clamp", "eq", "ne", "lt", "lte", "gt",
    "gte", "and", "or", "not", "if", "coalesce", "concat", "length", "sum", "average", "filter",
    "map", "pluck", "includes", "format",
];

pub(super) fn validate(value: &Value, depth: usize) -> Result<(), String> {
    if depth > 24 {
        return Err("expression depth exceeds 24".into());
    }
    match value {
        Value::Object(map) if map.contains_key("$get") => {
            let path = map["$get"].as_str().ok_or("$get requires a path")?;
            if map.len() != 1 || path.len() > 240 {
                return Err("invalid $get expression".into());
            }
            let pieces = path.split('.').collect::<Vec<_>>();
            if pieces.len() < 2
                || pieces.len() > 16
                || !matches!(pieces[0], "state" | "data" | "computed" | "item")
                || pieces[1..].iter().any(|k| !ui_identifier(k))
            {
                return Err(format!("invalid lookup {path}"));
            }
        }
        Value::Object(map) if map.contains_key("$op") => {
            let op = map["$op"].as_str().ok_or("$op requires a name")?;
            let args = map
                .get("args")
                .and_then(Value::as_array)
                .ok_or("operator requires args")?;
            if map.len() != 2 || !OPERATORS.contains(&op) || args.len() > 32 {
                return Err(format!("unsupported operation {op}"));
            }
            let valid_count = match op {
                "if" | "clamp" => args.len() == 3,
                "not" | "length" | "sum" | "average" => args.len() == 1,
                "sub" | "div" | "eq" | "ne" | "lt" | "lte" | "gt" | "gte" | "filter" | "map"
                | "pluck" | "includes" => args.len() == 2,
                "round" | "format" => (1..=2).contains(&args.len()),
                _ => (1..=32).contains(&args.len()),
            };
            if !valid_count {
                return Err(format!("wrong number of args for {op}"));
            }
            for arg in args {
                validate(arg, depth + 1)?;
            }
        }
        Value::Object(map) if map.contains_key("$literal") => {
            if map.len() != 1 {
                return Err("invalid literal expression".into());
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                if k.starts_with('$') {
                    return Err(format!("unknown expression key {k}"));
                }
                validate(v, depth + 1)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                validate(item, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) struct Evaluator<'a> {
    pub program: &'a UiProgram,
    state: &'a UiState,
    pub cache: UiState,
    active: BTreeSet<String>,
    steps: usize,
    depth: usize,
    produced_bytes: usize,
}
impl<'a> Evaluator<'a> {
    pub fn new(program: &'a UiProgram, state: &'a UiState) -> Self {
        Self {
            program,
            state,
            cache: UiState::new(),
            active: BTreeSet::new(),
            steps: 0,
            depth: 0,
            produced_bytes: 0,
        }
    }
    pub fn computed(&mut self, key: &str) -> Result<Value, String> {
        if let Some(value) = self.cache.get(key) {
            return Ok(value.clone());
        }
        if !self.active.insert(key.to_owned()) {
            return Err(format!("cyclic computed dependency {key}"));
        }
        let expr = self
            .program
            .computed
            .get(key)
            .ok_or_else(|| format!("unknown computed value {key}"))?
            .clone();
        let value = self.eval(&expr, &Value::Null)?;
        self.active.remove(key);
        self.cache.insert(key.into(), value.clone());
        Ok(value)
    }
    pub fn eval(&mut self, value: &Value, item: &Value) -> Result<Value, String> {
        self.steps += 1;
        self.depth += 1;
        if self.steps > UI_MAX_EVALUATION_STEPS || self.depth > 64 {
            return Err("UI evaluation budget exceeded".into());
        }
        let result = self.inner(value, item);
        self.depth -= 1;
        let result = result?;
        let mut bytes = 0;
        result_size(&result, &mut bytes, 0)?;
        self.produced_bytes = self.produced_bytes.saturating_add(bytes);
        if self.produced_bytes > MAX_PRODUCED_BYTES {
            return Err("UI result budget exceeded".into());
        }
        Ok(result)
    }
    fn inner(&mut self, value: &Value, item: &Value) -> Result<Value, String> {
        match value {
            Value::Object(map) if map.contains_key("$literal") => Ok(map["$literal"].clone()),
            Value::Object(map) if map.contains_key("$get") => {
                let path = map["$get"].as_str().ok_or("invalid lookup")?;
                let mut parts = path.split('.');
                let root = parts.next().unwrap_or("");
                let key = parts.next().unwrap_or("");
                let mut current = match root {
                    "state" => self.state.get(key).cloned(),
                    "data" => self.program.data.get(key).cloned(),
                    "computed" => Some(self.computed(key)?),
                    "item" => item.get(key).cloned(),
                    _ => None,
                }
                .unwrap_or(Value::Null);
                for key in parts {
                    current = current.get(key).cloned().unwrap_or(Value::Null);
                }
                Ok(current)
            }
            Value::Object(map) if map.contains_key("$op") => {
                let op = map["$op"].as_str().ok_or("invalid op")?;
                let args = map
                    .get("args")
                    .and_then(Value::as_array)
                    .ok_or("invalid args")?;
                if op == "if" {
                    return if boolean(&self.eval(&args[0], item)?)? {
                        self.eval(&args[1], item)
                    } else {
                        self.eval(&args[2], item)
                    };
                }
                if op == "and" || op == "or" {
                    for arg in args {
                        let v = boolean(&self.eval(arg, item)?)?;
                        if v == (op == "or") {
                            return Ok(json!(v));
                        }
                    }
                    return Ok(json!(op == "and"));
                }
                if op == "coalesce" {
                    for arg in args {
                        let v = self.eval(arg, item)?;
                        if !v.is_null() {
                            return Ok(v);
                        }
                    }
                    return Ok(Value::Null);
                }
                if op == "filter" || op == "map" {
                    let source = self.eval(&args[0], item)?;
                    let items = source.as_array().ok_or("filter/map expects array")?;
                    if items.len() > 1024 {
                        return Err("collection exceeds 1024 rows".into());
                    }
                    let mut result = Vec::new();
                    for row in items {
                        let v = self.eval(&args[1], row)?;
                        if op == "map" {
                            result.push(v);
                        } else if boolean(&v)? {
                            result.push(row.clone());
                        }
                    }
                    return Ok(Value::Array(result));
                }
                let values = args
                    .iter()
                    .map(|v| self.eval(v, item))
                    .collect::<Result<Vec<_>, _>>()?;
                match op {
                    "add" => number(
                        values
                            .iter()
                            .map(numeric)
                            .collect::<Result<Vec<_>, _>>()?
                            .iter()
                            .sum(),
                    ),
                    "mul" => number(
                        values
                            .iter()
                            .map(numeric)
                            .collect::<Result<Vec<_>, _>>()?
                            .iter()
                            .product(),
                    ),
                    "sub" => number(numeric(&values[0])? - numeric(&values[1])?),
                    "div" => {
                        let denominator = numeric(&values[1])?;
                        if denominator == 0.0 {
                            return Err("division by zero; guard with if/coalesce".into());
                        }
                        number(numeric(&values[0])? / denominator)
                    }
                    "min" | "max" => {
                        let numbers = values.iter().map(numeric).collect::<Result<Vec<_>, _>>()?;
                        number(
                            numbers
                                .into_iter()
                                .reduce(|a, b| if op == "min" { a.min(b) } else { a.max(b) })
                                .unwrap_or(0.0),
                        )
                    }
                    "round" | "format" => {
                        let digits = values.get(1).map(numeric).transpose()?.unwrap_or(0.0);
                        if !(0.0..=6.0).contains(&digits) || digits.fract() != 0.0 {
                            return Err("decimal digits must be an integer 0..6".into());
                        }
                        let scale = 10_f64.powi(digits as i32);
                        let n = (numeric(&values[0])? * scale).round() / scale;
                        if op == "format" {
                            Ok(json!(format!(
                                "{:.*}",
                                digits as usize,
                                if n == 0.0 { 0.0 } else { n }
                            )))
                        } else {
                            number(n)
                        }
                    }
                    "clamp" => {
                        let n = numeric(&values[0])?;
                        let low = numeric(&values[1])?;
                        let high = numeric(&values[2])?;
                        if low > high {
                            return Err("clamp minimum exceeds maximum".into());
                        }
                        number(n.max(low).min(high))
                    }
                    "eq" | "ne" => Ok(json!(if op == "eq" {
                        equal(&values[0], &values[1])
                    } else {
                        !equal(&values[0], &values[1])
                    })),
                    "lt" | "lte" | "gt" | "gte" => {
                        let a = numeric(&values[0])?;
                        let b = numeric(&values[1])?;
                        Ok(json!(match op {
                            "lt" => a < b,
                            "lte" => a <= b,
                            "gt" => a > b,
                            _ => a >= b,
                        }))
                    }
                    "not" => Ok(json!(!boolean(&values[0])?)),
                    "concat" => {
                        let text = values.iter().map(display).collect::<Vec<_>>().join("");
                        if text.len() > 32 * 1024 {
                            return Err("concatenated text exceeds 32 KiB".into());
                        }
                        Ok(json!(text))
                    }
                    "length" => match &values[0] {
                        Value::Array(v) => Ok(json!(v.len())),
                        Value::String(s) => Ok(json!(s.chars().count())),
                        _ => Err("length expects array or text".into()),
                    },
                    "sum" | "average" => {
                        let items = values[0].as_array().ok_or("sum/average expects array")?;
                        let sum = items
                            .iter()
                            .map(numeric)
                            .collect::<Result<Vec<_>, _>>()?
                            .iter()
                            .sum::<f64>();
                        number(if op == "average" {
                            if items.is_empty() {
                                0.0
                            } else {
                                sum / items.len() as f64
                            }
                        } else {
                            sum
                        })
                    }
                    "pluck" => {
                        let rows = values[0].as_array().ok_or("pluck expects array")?;
                        let key = values[1]
                            .as_str()
                            .filter(|s| ui_identifier(s))
                            .ok_or("pluck expects a safe field name")?;
                        Ok(Value::Array(
                            rows.iter()
                                .map(|r| r.get(key).cloned().unwrap_or(Value::Null))
                                .collect(),
                        ))
                    }
                    "includes" => match &values[0] {
                        Value::Array(items) => {
                            Ok(json!(items.iter().any(|v| equal(v, &values[1]))))
                        }
                        Value::String(s) => Ok(json!(
                            s.contains(values[1].as_str().ok_or("text includes expects text")?)
                        )),
                        _ => Err("includes expects array or text".into()),
                    },
                    _ => Err(format!("unsupported operator {op}")),
                }
            }
            Value::Object(map) => {
                let mut result = serde_json::Map::new();
                for (k, v) in map {
                    result.insert(k.clone(), self.eval(v, item)?);
                }
                Ok(Value::Object(result))
            }
            Value::Array(items) => Ok(Value::Array(
                items
                    .iter()
                    .map(|v| self.eval(v, item))
                    .collect::<Result<_, _>>()?,
            )),
            _ => Ok(value.clone()),
        }
    }
}
fn equal(a: &Value, b: &Value) -> bool {
    if a.is_number() && b.is_number() {
        a.as_f64() == b.as_f64()
    } else {
        a == b
    }
}
fn numeric(v: &Value) -> Result<f64, String> {
    v.as_f64()
        .filter(|n| n.is_finite() && n.abs() <= 1e100)
        .ok_or_else(|| "expected a finite number".into())
}
fn boolean(v: &Value) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| "expected a boolean".into())
}
fn number(n: f64) -> Result<Value, String> {
    if !n.is_finite() || n.abs() > 1e100 {
        return Err("numeric result exceeds safe range".into());
    }
    Ok(json!(if n == 0.0 { 0.0 } else { n }))
}
pub(super) fn display(v: &Value) -> String {
    match v {
        Value::Null => "—".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n
            .as_f64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| n.to_string()),
        _ => v.to_string(),
    }
}
