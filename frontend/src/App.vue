<template>
  <div id="app">
    <router-view />
    <ToastNotification />
  </div>
</template>

<script setup>
import ToastNotification from '@/components/ToastNotification.vue'
</script>

<style>
/* Global styles are imported in main.js */
</style>
