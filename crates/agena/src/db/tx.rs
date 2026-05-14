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

    async fn run(self) {
        for effect in self.effects {
            effect.await;
        }
    }
}

pub async fn with_transaction_and_effects<T, O>(db: &DatabaseConnection, op: O) -> Result<T, DbErr>
where
    O: for<'a> FnOnce(
        &'a DatabaseTransaction,
        &'a mut DbTxEffects,
    ) -> Pin<Box<dyn Future<Output = Result<T, DbErr>> + Send + 'a>>,
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

pub async fn with_transaction_and_app_effects<T, E, O>(
    db: &DatabaseConnection,
    op: O,
) -> Result<T, E>
where
    E: From<DbErr>,
    O: for<'a> FnOnce(
        &'a DatabaseTransaction,
        &'a mut DbTxEffects,
    ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>,
{
    let txn = db.begin().await.map_err(E::from)?;
    let mut effects = DbTxEffects::default();

    let result = op(&txn, &mut effects).await;
    match result {
        Ok(value) => {
            txn.commit().await.map_err(E::from)?;
            effects.run().await;
            Ok(value)
        }
        Err(err) => {
            txn.rollback().await.map_err(E::from)?;
            Err(err)
        }
    }
}
