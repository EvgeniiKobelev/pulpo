use crate::types::*;
use std::pin::Pin;
use futures::Stream;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    OrderBook(OrderBook),
    Trade(Trade),
    Candle(Candle),
    Ticker(Ticker),
    FundingRate(FundingRate),
    MarkPrice(MarkPrice),
    Liquidation(Liquidation),
    Info(String),
}

pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

pub struct Subscription {
    _cancel: tokio::sync::oneshot::Sender<()>,
}

impl Subscription {
    pub fn new(cancel: tokio::sync::oneshot::Sender<()>) -> Self {
        Self { _cancel: cancel }
    }
}

/// Следующее сообщение `rx`, пока потребитель `out` жив.
///
/// Таски-пересыльщики (`raw → out`) ждут `rx.recv()` вечно, если биржа
/// молчит, и держат сырой канал открытым — WS-таск не узнаёт, что стрим
/// бросили, и крутит соединение с реконнектами до рестарта процесса.
/// `None` — и когда `rx` закрыт, и когда потребитель `out` ушёл: в обоих
/// случаях пересыльщику пора выходить.
pub async fn recv_while_open<T, U>(
    rx: &mut tokio::sync::mpsc::Receiver<T>,
    out: &tokio::sync::mpsc::Sender<U>,
) -> Option<T> {
    let closed = out.closed();
    let recv = rx.recv();
    futures::pin_mut!(closed, recv);
    // `select` опрашивает левую ветку первой — закрытый выход важнее
    // готового сообщения.
    match futures::future::select(closed, recv).await {
        futures::future::Either::Left(_) => None,
        futures::future::Either::Right((msg, _)) => msg,
    }
}

/// То же, что [`recv_while_open`], для произвольного стрима.
pub async fn next_while_open<S, U>(
    stream: &mut S,
    out: &tokio::sync::mpsc::Sender<U>,
) -> Option<S::Item>
where
    S: Stream + Unpin,
{
    use futures::StreamExt;
    let closed = out.closed();
    let next = stream.next();
    futures::pin_mut!(closed, next);
    match futures::future::select(closed, next).await {
        futures::future::Either::Left(_) => None,
        futures::future::Either::Right((item, _)) => item,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn forwarder_exits_when_consumer_drops_on_silent_source() {
        let (_raw_tx, mut raw_rx) = mpsc::channel::<u32>(4);
        let (out_tx, out_rx) = mpsc::channel::<u32>(4);
        drop(out_rx);
        // Источник молчит, но выход закрыт — пересыльщик не должен висеть.
        assert_eq!(recv_while_open(&mut raw_rx, &out_tx).await, None);
    }

    #[tokio::test]
    async fn forwarder_passes_messages_while_consumer_alive() {
        let (raw_tx, mut raw_rx) = mpsc::channel::<u32>(4);
        let (out_tx, _out_rx) = mpsc::channel::<u32>(4);
        raw_tx.send(7).await.unwrap();
        assert_eq!(recv_while_open(&mut raw_rx, &out_tx).await, Some(7));
        let mut s = futures::stream::iter([1u32]);
        assert_eq!(next_while_open(&mut s, &out_tx).await, Some(1));
    }
}
