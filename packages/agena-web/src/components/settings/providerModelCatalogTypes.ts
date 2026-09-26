export type CatalogPickerModel = {
  model_id: string
  display_name?: string | null
  origin?: string | null
  description?: string | null
  context_window_tokens?: number | null
  max_output_tokens?: number | null
}
