//! Локальный стакан per-символ для Binance Spot/Futures.
//!
//! Используется для maintain'а консистентного 1000-уровневого orderbook'а
//! поверх @depth@100ms diff-стрима по стандартной схеме Binance:
//!
//! 1. Буферим WS diff-события.
//! 2. Запрашиваем REST snapshot (limit=1000).
//! 3. Отбрасываем буферизованные events до snapshot.lastUpdateId.
//! 4. Применяем валидные diff'ы поверх локального стакана.
//! 5. На разрывах sequence id → re-sync (заново REST snapshot).
//!
//! После maintain'а отдаём в выходной стрим:
//! * на каждый WS diff — `OrderBook` с пришедшими уровнями (как было раньше,
//!   но уже из консистентной локальной книги — qty=0 для удалений).
//! * на initial sync — `OrderBook` со всеми top-1000 уровнями локальной книги.
//! * на re-sync — diff между старой и новой книгой
//!   (удалённые уровни как qty=0, новые/изменённые с их qty).

use gateway_core::{ExchangeId, Level, OrderBook, Symbol};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// Локальная книга для одного символа.
#[derive(Debug, Clone, Default)]
pub struct LocalOrderBook {
    /// Стороны: цена → qty. BTreeMap для упорядоченности.
    pub bids: BTreeMap<Decimal, Decimal>,
    pub asks: BTreeMap<Decimal, Decimal>,
    /// last_update_id из последнего применённого события.
    pub last_update_id: u64,
    /// Готов ли стакан к emit'у (true после initial snapshot + validate).
    pub ready: bool,
}

impl LocalOrderBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Установить состояние из REST snapshot. Перезаписывает всё.
    pub fn set_snapshot(
        &mut self,
        bids: impl IntoIterator<Item = (Decimal, Decimal)>,
        asks: impl IntoIterator<Item = (Decimal, Decimal)>,
        last_update_id: u64,
    ) {
        self.bids.clear();
        self.asks.clear();
        for (price, qty) in bids {
            if qty > Decimal::ZERO {
                self.bids.insert(price, qty);
            }
        }
        for (price, qty) in asks {
            if qty > Decimal::ZERO {
                self.asks.insert(price, qty);
            }
        }
        self.last_update_id = last_update_id;
        self.ready = true;
    }

    /// Применить diff к локальной книге. qty=0 удаляет уровень.
    /// Не делает sequence id проверку — её делает caller.
    pub fn apply_diff(
        &mut self,
        bids: impl IntoIterator<Item = (Decimal, Decimal)>,
        asks: impl IntoIterator<Item = (Decimal, Decimal)>,
        new_update_id: u64,
    ) {
        for (price, qty) in bids {
            if qty.is_zero() {
                self.bids.remove(&price);
            } else {
                self.bids.insert(price, qty);
            }
        }
        for (price, qty) in asks {
            if qty.is_zero() {
                self.asks.remove(&price);
            } else {
                self.asks.insert(price, qty);
            }
        }
        self.last_update_id = new_update_id;
    }

    /// Top-N уровней с обеих сторон.
    /// Bids: descending (от высокой цены к низкой).
    /// Asks: ascending (от низкой цены к высокой).
    pub fn top_levels(&self, n: usize) -> (Vec<Level>, Vec<Level>) {
        let bids: Vec<Level> = self
            .bids
            .iter()
            .rev()
            .take(n)
            .map(|(p, q)| Level::new(*p, *q))
            .collect();
        let asks: Vec<Level> = self
            .asks
            .iter()
            .take(n)
            .map(|(p, q)| Level::new(*p, *q))
            .collect();
        (bids, asks)
    }

    /// Полный snapshot книги как `OrderBook` (top-1000 уровней) для emit'а
    /// после initial sync. Этот OrderBook совместим с density `on_orderbook_delta`:
    /// все уровни upsert'нутся как новые плотности.
    pub fn to_orderbook(
        &self,
        exchange: ExchangeId,
        symbol: Symbol,
        timestamp_ms: u64,
        top_n: usize,
    ) -> OrderBook {
        let (bids, asks) = self.top_levels(top_n);
        OrderBook {
            exchange,
            symbol,
            bids,
            asks,
            timestamp_ms,
            sequence: Some(self.last_update_id),
        }
    }

    /// Diff между прежним и текущим состоянием книги.
    /// Возвращает уровни, которые нужно отправить как delta:
    /// * удалённые уровни (есть в prev, нет в self) → Level { price, qty: 0 }
    /// * новые уровни (есть в self, нет в prev) → Level { price, qty }
    /// * изменённые уровни (qty отличается) → Level { price, qty }
    ///
    /// Ограничено топ-N уровнями каждой книги, чтобы не отдавать миллион уровней.
    pub fn diff_against_prev(
        &self,
        prev: &LocalOrderBook,
        top_n: usize,
    ) -> (Vec<Level>, Vec<Level>) {
        let bids = diff_side(&prev.bids, &self.bids, top_n, /*reverse=*/ true);
        let asks = diff_side(&prev.asks, &self.asks, top_n, /*reverse=*/ false);
        (bids, asks)
    }
}

/// Diff одной стороны. Берём top-N уровней объединения и для каждого
/// сравниваем prev.qty vs new.qty.
fn diff_side(
    prev: &BTreeMap<Decimal, Decimal>,
    new: &BTreeMap<Decimal, Decimal>,
    top_n: usize,
    reverse: bool,
) -> Vec<Level> {
    // Берём top-N уровней из new (это то, что мы хотим видеть как
    // "текущее" состояние), плюс уровни prev, которых нет в new — для emit
    // qty=0 (delete). Чтобы не раздувать — ограничиваем top-N из prev тоже.
    let mut candidates: BTreeMap<Decimal, ()> = BTreeMap::new();
    let new_iter: Box<dyn Iterator<Item = (&Decimal, &Decimal)>> = if reverse {
        Box::new(new.iter().rev())
    } else {
        Box::new(new.iter())
    };
    for (p, _) in new_iter.take(top_n) {
        candidates.insert(*p, ());
    }
    let prev_iter: Box<dyn Iterator<Item = (&Decimal, &Decimal)>> = if reverse {
        Box::new(prev.iter().rev())
    } else {
        Box::new(prev.iter())
    };
    for (p, _) in prev_iter.take(top_n) {
        candidates.insert(*p, ());
    }

    let mut out: Vec<Level> = Vec::with_capacity(candidates.len());
    for (price, _) in &candidates {
        let new_qty = new.get(price).copied().unwrap_or(Decimal::ZERO);
        let prev_qty = prev.get(price).copied().unwrap_or(Decimal::ZERO);
        if new_qty != prev_qty {
            out.push(Level::new(*price, new_qty));
        }
    }
    // Сортируем как и положено: bids desc, asks asc.
    if reverse {
        out.sort_by(|a, b| b.price.cmp(&a.price));
    } else {
        out.sort_by(|a, b| a.price.cmp(&b.price));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn apply_diff_qty_zero_removes_level() {
        let mut book = LocalOrderBook::new();
        book.set_snapshot(
            vec![(dec!(100.0), dec!(1.5)), (dec!(99.0), dec!(2.0))],
            vec![(dec!(101.0), dec!(0.5))],
            10,
        );
        assert_eq!(book.bids.len(), 2);

        book.apply_diff(
            vec![(dec!(99.0), Decimal::ZERO)],
            vec![],
            11,
        );
        assert_eq!(book.bids.len(), 1);
        assert!(book.bids.contains_key(&dec!(100.0)));
        assert_eq!(book.last_update_id, 11);
    }

    #[test]
    fn top_levels_order() {
        let mut book = LocalOrderBook::new();
        book.set_snapshot(
            vec![
                (dec!(98.0), dec!(1.0)),
                (dec!(100.0), dec!(2.0)),
                (dec!(99.0), dec!(1.5)),
            ],
            vec![
                (dec!(103.0), dec!(1.0)),
                (dec!(101.0), dec!(0.5)),
                (dec!(102.0), dec!(0.7)),
            ],
            1,
        );
        let (bids, asks) = book.top_levels(10);
        // bids descending
        assert_eq!(bids[0].price, dec!(100.0));
        assert_eq!(bids[1].price, dec!(99.0));
        assert_eq!(bids[2].price, dec!(98.0));
        // asks ascending
        assert_eq!(asks[0].price, dec!(101.0));
        assert_eq!(asks[1].price, dec!(102.0));
        assert_eq!(asks[2].price, dec!(103.0));
    }

    #[test]
    fn diff_against_prev_emits_zero_for_deleted_levels() {
        let mut prev = LocalOrderBook::new();
        prev.set_snapshot(
            vec![(dec!(100.0), dec!(1.0)), (dec!(99.0), dec!(2.0))],
            vec![(dec!(101.0), dec!(0.5))],
            1,
        );
        let mut new = LocalOrderBook::new();
        new.set_snapshot(
            vec![(dec!(100.0), dec!(1.5))],
            vec![(dec!(101.0), dec!(0.5)), (dec!(102.0), dec!(0.3))],
            2,
        );
        let (bids, asks) = new.diff_against_prev(&prev, 100);
        // bids: 100 (1.0 → 1.5), 99 (2.0 → 0)
        assert_eq!(bids.len(), 2);
        assert_eq!(bids[0].price, dec!(100.0));
        assert_eq!(bids[0].qty, dec!(1.5));
        assert_eq!(bids[1].price, dec!(99.0));
        assert_eq!(bids[1].qty, Decimal::ZERO);
        // asks: 101 unchanged, 102 new (Decimal::ZERO → 0.3)
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].price, dec!(102.0));
        assert_eq!(asks[0].qty, dec!(0.3));
    }
}
