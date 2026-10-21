<script setup lang="ts">
import { toRefs, ref } from 'vue';
import PermissionsPanel from './PermissionsPanel.vue';
import UpdatePanel from './UpdatePanel.vue';
import thirdPartyNotices from '../../THIRD_PARTY_NOTICES.txt?raw';
import type { UnwrapRef } from 'vue';
import type { VaultUI } from '../useVault';
const props = defineProps<{ v: UnwrapRef<VaultUI> }>();
const { info, configVisible, config, settingsSaving } = toRefs(props.v);
const { updateSettings, toggleStartup, copyConfig } = props.v;
const tab=ref<'connection'|'permissions'>('permissions');
const licenses=ref<HTMLDialogElement|null>(null);
</script>
<template>
  <section class="settings" :class="{'mcp-settings':v.page==='mcp'}">
    <div v-if="v.page==='mcp'">
      <div class="setting-row" style="padding-top:0"><div>允许 MCP 访问<small>关闭后，已连接客户端的后续请求也会被拒绝</small></div><button id="mcp-toggle" class="switch" :class="{off:!info?.settings.mcpEnabled}" role="switch" :aria-checked="info?.settings.mcpEnabled" aria-label="允许 MCP 访问" :disabled="settingsSaving||!info" @click="updateSettings('mcpEnabled')"></button></div>
      <nav class="mcp-tabs" role="tablist" aria-label="MCP 管理"><button id="connection-tab" role="tab" :aria-selected="tab==='connection'" :class="{active:tab==='connection'}" @click="tab='connection'">连接配置</button><button id="permissions-tab" role="tab" :aria-selected="tab==='permissions'" :class="{active:tab==='permissions'}" @click="tab='permissions'">权限管理</button></nav>
      <div v-show="tab==='connection'"><h2>把凭据库连接到 AI</h2>
      <p class="description">AI 客户端按需启动独立进程，桌面窗口关闭后也可以使用。</p>
      <div class="card">
        <h3>MCP 访问</h3>
        <div class="setting-row"><div>连接方式<small>客户端启动本地程序，不监听网络端口</small></div><span class="pill">stdio · 按需启动</span></div>
        <div class="setting-row"><div>客户端配置<small>路径已按当前程序位置生成</small></div><button id="show-config" class="secondary" @click="configVisible=!configVisible">{{configVisible?'收起':'查看配置'}}</button></div>
        <div v-if="configVisible"><pre id="mcp-config">{{config}}</pre><button id="copy-config" class="primary" @click="copyConfig">复制配置</button></div>
      </div>
      <p class="description">本软件的 AI 输出不返回凭据原文，备注默认不外发。凭据填写到目标后，其他程序仍可能读取目标内容。输入事件已发送不代表填写或登录成功。</p>
      </div>
      <PermissionsPanel v-show="tab==='permissions'" :v="v"/>
    </div>
    <div v-else>
      <h2>按照你的习惯运行</h2>
      <p class="description">解锁随当前 Windows 身份完成，无需额外输入主密码。</p>
      <div class="card">
        <h3>启动与窗口</h3>
        <div class="setting-row"><div>登录 Windows 后启动<small>可选；移动程序后需重新开启以更新路径</small></div><button id="startup-toggle" class="switch" :class="{off:!info?.startupEnabled}" role="switch" :aria-checked="info?.startupEnabled" aria-label="登录 Windows 后启动" @click="toggleStartup"></button></div>
        <div class="setting-row"><div>关闭窗口时保留在托盘<small>从托盘菜单可重新打开或彻底退出</small></div><button id="tray-toggle" class="switch" :class="{off:!info?.settings.closeToTray}" role="switch" :aria-checked="info?.settings.closeToTray" aria-label="关闭窗口时保留在托盘" :disabled="settingsSaving||!info" @click="updateSettings('closeToTray')"></button></div>
      </div>
      <div class="card">
        <h3>本地密码库</h3>
        <div class="setting-row"><span>保护方式</span><span class="pill">本地加密 · Windows 用户身份</span></div>
        <div class="setting-row vertical"><span>数据目录</span><code id="data-directory">{{info?.dataDirectory}}</code><small>请保留完整 data 目录。该密码库绑定创建它的 Windows 账户与电脑。</small></div>
        <div class="setting-row"><span>应用版本</span><span>{{info?.version}}</span></div>
        <div class="setting-row"><span>开源许可</span><button class="text-btn" @click="licenses?.showModal()">查看第三方声明</button></div>
      </div>
      <UpdatePanel :v="v"/>
    </div>
    <dialog ref="licenses" class="licenses-dialog"><h2>第三方开源声明</h2><pre>{{thirdPartyNotices}}</pre><div class="modal-actions"><button class="secondary" autofocus @click="licenses?.close()">关闭</button></div></dialog>
  </section>
</template>
<style>
.settings.mcp-settings{max-width:none;min-width:0}.mcp-tabs{display:flex;gap:26px;border-bottom:1px solid #e7eaf1;margin:6px 0 25px}.mcp-tabs button{position:relative;background:none;border:0;font:inherit;font-size:12px;color:#8994a9;padding:14px 0}.mcp-tabs button.active{color:#5364dc;font-weight:600}.mcp-tabs button.active:after{content:'';position:absolute;left:0;right:0;bottom:-1px;height:2px;background:#5364dc}
.licenses-dialog{width:min(800px,85vw)}.licenses-dialog pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:50vh;overflow:auto;font-size:11px}
</style>
