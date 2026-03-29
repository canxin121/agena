use std::future::Future;
use std::pin::Pin;

use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};

type DbEffectFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Default)]
pub struct DbTxEffects {
    effects: Vec<DbEffectFuture>,
}

impl DbTxEffects {
    pub fn push<F>(&mut self, effect: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.effects.push(Box::pin(effect));
    }

    async fn run(mut self) {
        while let Some(effect) = self.effects.pop() {
            effect.await;
        }
    }
}

pub async fn with_transaction_and_effects<T, O, Fut>(
    db: &DatabaseConnection,
    op: O,
) -> Result<T, DbErr>
where
    O: FnOnce(&DatabaseTransaction, &mut DbTxEffects) -> Fut,
    Fut: Future<Output = Result<T, DbErr>>,
{
    let txn = db.begin().await?;
    let mut effects = DbTxEffects::default();

    let result = op(&txn, &mut effects).await;
    match result {
        Ok(value) => {
            txn.commit().await?;
            effects.run().await;
            Ok(value)
        }
        Err(err) => {
            txn.rollback().await?;
            Err(err)
        }
    }
}
