//! Read slots are independent of mutation ownership; navigation does no paid work.
use super::*;
use crate::{
    library::{Details, Game, Page, Query},
    storage::library,
};
impl Controller {
    pub async fn warhammer_books(&self, query: Query) -> UiResult<Page> {
        let (_permit, db) = self.catalogue_reader()?;
        let mut tx = db.pool.begin().await.map_err(message)?;
        let result = library::page(&mut tx, &query).await.map_err(message)?;
        tx.commit().await.map_err(message)?;
        Ok(result)
    }
    pub async fn warhammer_book(
        &self,
        game: Game,
        target: String,
        product_id: String,
    ) -> UiResult<Option<Details>> {
        let (_permit, db) = self.catalogue_reader()?;
        let mut conn = db.pool.acquire().await.map_err(message)?;
        library::details(&mut conn, game, &target, &product_id)
            .await
            .map_err(message)
    }
}
