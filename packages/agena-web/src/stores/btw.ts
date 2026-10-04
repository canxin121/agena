import { defineStore } from 'pinia'
import { askBtw } from '../pages/chat/btwRequest'
import { useBtw } from '../pages/chat/btwState'

export const useBtwStore = defineStore('btw', () => useBtw(askBtw))
