//! Кэш `ctVal` (contract value) per-instrument для OKX SWAP.
//!
//! У OKX все perpetual swap'ы являются контрактами с фиксированным размером
//! в базовой валюте. Например `BTC-USDT-SWAP` имеет `ctVal=0.01` BTC — то
//! есть 1 «контракт» в стакане = 0.01 BTC. Без домножения на ctVal density
//! принимает sz как количество монет напрямую и считает нотинал
//! `price × sz` — что для BTC завышает реальный объём в 100 раз.
//!
//! Этот модуль один раз тянет `/api/v5/public/instruments?instType=SWAP`
//! и кэширует `ctVal` per `Symbol`. Кэш — глобальный (`OnceCell`),
//! инициализируется лениво при первом вызове `ct_val_for`.

use crate::futures::mapper::OkxSwapInstrumentRaw;
use crate::spot::mapper::{okx_inst_id_to_unified, OkxResponse};
use gateway_core::*;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::{info, warn};

static CT_VALS: OnceCell<HashMap<Symbol, Decimal>> = OnceCell::const_new();

const URL: &str = "https://www.okx.com/api/v5/public/instruments?instType=SWAP";

async fn load() -> HashMap<Symbol, Decimal> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "OKX ct_vals: failed to build HTTP client");
            return HashMap::new();
        }
    };
    let resp = match client.get(URL).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "OKX ct_vals: failed to fetch instruments");
            return HashMap::new();
        }
    };
    let body: OkxResponse<OkxSwapInstrumentRaw> = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "OKX ct_vals: failed to parse instruments response");
            return HashMap::new();
        }
    };
    if body.code != "0" {
        warn!(code = %body.code, msg = %body.msg, "OKX ct_vals: non-zero code");
        return HashMap::new();
    }

    let mut map = HashMap::new();
    for inst in body.data {
        // Только linear USDT-margined. Inverse (BTC-USD-SWAP) считается
        // иначе и в density не используется.
        if inst.ct_type != "linear" {
            continue;
        }
        let Some(symbol) = okx_inst_id_to_unified(&inst.inst_id) else {
            continue;
        };
        let Ok(val) = Decimal::from_str(&inst.ct_val) else {
            continue;
        };
        if val > Decimal::ZERO {
            map.insert(symbol, val);
        }
    }
    info!(count = map.len(), "OKX ct_vals: loaded contract values");
    map
}

/// Возвращает ctVal для символа. На первом вызове тянет `/instruments` и
/// кэширует. На последующих — мгновенно отдаёт значение из памяти.
///
/// Если символ не найден (новый instrument, не USDT-margined и т.п.) или
/// REST-вызов упал — возвращает `1`, что эквивалентно «множитель не нужен».
pub async fn ct_val_for(symbol: &Symbol) -> Decimal {
    let map = CT_VALS.get_or_init(load).await;
    map.get(symbol).copied().unwrap_or(Decimal::ONE)
}

/// Прогревочный вызов: можно дёрнуть из stream_orderbooks_batch один раз,
/// чтобы загрузка случилась до начала потока ws-событий (а не на первом).
pub async fn warm_up() {
    let _ = CT_VALS.get_or_init(load).await;
}
