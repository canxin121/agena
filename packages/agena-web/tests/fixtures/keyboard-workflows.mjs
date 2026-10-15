import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { i18n } from '../../src/i18n'
import { router } from '../../src/router'
import { installImeCompositionGuard } from '../../src/lib/imeKeyboard'
import Workflows from './keyboard-workflows.vue'
import '../../src/style.css'
i18n.global.locale.value = 'zh-CN'
document.documentElement.classList.add('dark')
installImeCompositionGuard()
createApp(Workflows).use(createPinia()).use(i18n).use(router).mount('#app')
