// Pin the message locale for `bun test`.
//
// The browser default is zh-CN, but tests assert the English source copy of
// user-facing errors and labels. Pinning en-US keeps assertions stable now that
// those strings resolve through vue-i18n instead of hardcoded literals.
import { i18n } from '../src/i18n'

i18n.global.locale.value = 'en-US'
