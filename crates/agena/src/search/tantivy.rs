use tantivy::{
    Index, TantivyDocument, TantivyError,
    schema::{Field, IndexRecordOption, TextFieldIndexing, TextOptions, Value},
    tokenizer::{LowerCaser, NgramTokenizer, SimpleTokenizer, TextAnalyzer},
};

pub(crate) fn indexed_text_options() -> TextOptions {
    TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}

pub(crate) fn ngram_text_options(tokenizer: &str) -> TextOptions {
    TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(tokenizer)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}

pub(crate) fn register_text_tokenizers(
    index: &Index,
    ngram_tokenizer: &str,
) -> Result<(), TantivyError> {
    let ngrams = TextAnalyzer::builder(
        NgramTokenizer::new(2, 4, false)
            .map_err(|err| TantivyError::InvalidArgument(err.to_string()))?,
    )
    .filter(LowerCaser)
    .build();
    let simple = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();
    index.tokenizers().register("default", simple);
    index.tokenizers().register(ngram_tokenizer, ngrams);
    Ok(())
}

pub(crate) fn first_text(doc: &TantivyDocument, field: Field) -> String {
    optional_text(doc, field).unwrap_or_default()
}

pub(crate) fn optional_text(doc: &TantivyDocument, field: Field) -> Option<String> {
    doc.get_first(field)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}
