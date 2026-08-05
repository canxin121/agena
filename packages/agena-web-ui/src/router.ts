import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'

import {
  buildRuntimeSectionPath,
  resolveRuntimeTabFromRoute,
  sanitizeRuntimeSectionQuery,
  sectionBasePaths,
  sectionPageLoaders,
} from './agena/pages/runtimePageStateModel'

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
    path: '/auth/callback',
    component: () => import('./agena/pages/AuthCallbackPage.vue'),
  },
  {
    path: '/workspace',
    component: () => import('./agena/pages/WorkspacePage.vue'),
  },
    {
    path: '/usage',
    component: () => import('./agena/pages/UsagePage.vue'),
  },
  {
    path: '/activities',
    component: () => import('./agena/pages/ActivitiesPage.vue'),
  },
  {
    path: sectionBasePaths.runtime,
    redirect: (to) => ({
      path: buildRuntimeSectionPath('runtime', resolveRuntimeTabFromRoute(to.path, to.query, 'runtime')),
      query: sanitizeRuntimeSectionQuery(to.query),
    }),
  },
  {
    path: `${sectionBasePaths.runtime}/:tab`,
    component: sectionPageLoaders.runtime,
  },
  {
    path: sectionBasePaths.plugins,
    redirect: (to) => ({
      path: buildRuntimeSectionPath('plugins', resolveRuntimeTabFromRoute(to.path, to.query, 'plugins')),
      query: sanitizeRuntimeSectionQuery(to.query),
    }),
  },
  {
    path: `${sectionBasePaths.plugins}/:tab`,
    component: sectionPageLoaders.plugins,
  },
  {
    path: sectionBasePaths.settings,
    redirect: (to) => ({
      path: buildRuntimeSectionPath('settings', resolveRuntimeTabFromRoute(to.path, to.query, 'settings')),
      query: sanitizeRuntimeSectionQuery(to.query),
    }),
  },
  {
    path: `${sectionBasePaths.settings}/:tab`,
    component: sectionPageLoaders.settings,
  },
]

export const router = createRouter({
  history: createWebHistory(),
  routes,
})
