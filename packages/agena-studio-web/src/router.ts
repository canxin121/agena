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
    redirect: '/settings',
  },
  {
    path: '/settings',
    component: () => import('./agena/pages/RuntimePage.vue'),
  },
  {
    path: '/workspace',
    component: () => import('./agena/pages/WorkspacePage.vue'),
  },
]

export const router = createRouter({
  history: createWebHistory(),
  routes,
})
