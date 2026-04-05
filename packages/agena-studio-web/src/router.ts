import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'

const routes: RouteRecordRaw[] = [
  {
    path: '/',
    redirect: '/chat',
  },
  {
    path: '/chat',
    component: () => import('./agena/pages/ChatPage.vue'),
  },
  {
    path: '/runtime',
    component: () => import('./agena/pages/RuntimePage.vue'),
  },
]

export const router = createRouter({
  history: createWebHistory(),
  routes,
})
