<script setup lang="ts">
// 全屏统一 loading：引导流程（检测/安装/启动）、内嵌页服务器启停与 DSH 更新、
// 鲸仔自更新共用。布局自上而下：居中 logo → 鲸仔 Whalito → DeepSeek Harness
// 桌面版 → 进度条 → 状态文字（可选次要细节行 / 可选已用时计时器）。
defineProps<{
  /** 主状态行，如「启动服务器中」「更新 DeepSeek Harness 中」。 */
  status: string;
  /** 次要细节行（可选），如 DSH 更新的「正在下载…」。 */
  detail?: string;
  /** 已用时计时器（可选），如 DSH 更新的「已用时 01:23」；为空则不显示。 */
  timer?: string;
}>();
</script>

<template>
  <div class="loading-screen">
    <img class="loading-logo" src="/logo.png" alt="鲸仔" />
    <p class="loading-name">鲸仔 <span class="loading-name-en">Whalito</span></p>
    <p class="loading-sub">DeepSeek Harness 桌面版</p>
    <div class="loading-track">
      <div class="loading-bar"></div>
    </div>
    <p class="loading-status">{{ status }}</p>
    <p v-if="detail" class="loading-detail">{{ detail }}</p>
    <p v-if="timer" class="loading-timer">{{ timer }}</p>
  </div>
</template>
