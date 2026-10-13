<script setup lang="ts">
import {computed,nextTick,onMounted,onUnmounted,ref,useId,watch} from 'vue';
const props=defineProps<{modelValue:string;options:readonly {value:string;label:string}[];disabled?:boolean;label:string}>();
const emit=defineEmits<{ 'update:modelValue':[value:string] }>();
const open=ref(false),active=ref(0),root=ref<HTMLElement|null>(null),trigger=ref<HTMLButtonElement|null>(null),list=ref<HTMLElement|null>(null);
const id=useId();const position=ref({left:'0px',top:'0px',width:'0px',maxHeight:'280px'});
const selected=computed(()=>props.options.find(o=>o.value===props.modelValue)?.label ?? '请选择');
async function reveal(){await nextTick();list.value?.children[active.value]?.scrollIntoView({block:'nearest'});}
async function expand(){
 if(props.disabled || !props.options.length)return;
 const rect=trigger.value?.getBoundingClientRect();if(!rect)return;
 const below=window.innerHeight-rect.bottom-10,above=rect.top-10;
 const height=Math.min(280,Math.max(80,below>=above?below:above),props.options.length*38+10);
 position.value={left:'0px',top:'0px',width:`${rect.width}px`,maxHeight:`${height}px`};
 position.value.left=`${Math.max(8,Math.min(rect.left,window.innerWidth-rect.width-8))}px`;
 position.value.top=`${below>=height?rect.bottom+5:Math.max(8,rect.top-height-5)}px`;
 active.value=Math.max(0,props.options.findIndex(o=>o.value===props.modelValue));open.value=true;await reveal();
}
function choose(index:number){const option=props.options[index];if(!option)return;emit('update:modelValue',option.value);open.value=false;trigger.value?.focus();}
function keydown(event:KeyboardEvent){
 if(props.disabled)return;
 if(['ArrowDown','ArrowUp','Home','End','Enter',' ','Escape'].includes(event.key))event.preventDefault();
 if(event.key==='Escape'){open.value=false;return;}if(event.key==='Tab'){open.value=false;return;}
 if(!open.value){if(['ArrowDown','ArrowUp','Enter',' '].includes(event.key))void expand();return;}
 if(event.key==='Enter'||event.key===' '){choose(active.value);return;}
 if(event.key==='Home')active.value=0;else if(event.key==='End')active.value=props.options.length-1;
 else if(event.key==='ArrowDown')active.value=Math.min(props.options.length-1,active.value+1);else if(event.key==='ArrowUp')active.value=Math.max(0,active.value-1);
 void reveal();
}
function outside(event:Event){const node=event.target as Node;if(!root.value?.contains(node)&&!list.value?.contains(node))open.value=false;}
function scrolling(event:Event){if(open.value && event.target!==list.value)open.value=false;}
function resized(){open.value=false;}
watch(()=>props.disabled,disabled=>{if(disabled)open.value=false;});
onMounted(()=>{document.addEventListener('pointerdown',outside,true);document.addEventListener('scroll',scrolling,true);window.addEventListener('resize',resized);});
onUnmounted(()=>{document.removeEventListener('pointerdown',outside,true);document.removeEventListener('scroll',scrolling,true);window.removeEventListener('resize',resized);});
</script>
<template>
 <div ref="root" class="theme-select"><button ref="trigger" class="select-trigger" type="button" role="combobox" :aria-label="label" :aria-expanded="open" :aria-controls="id" aria-haspopup="listbox" :aria-activedescendant="open?`${id}-${active}`:undefined" :disabled="disabled" @click="open?open=false:expand()" @keydown="keydown"><span>{{ selected }}</span><i class="select-arrow" :class="{open}"/></button>
 <Teleport to="body"><div v-if="open" ref="list" :id="id" class="select-popup" :style="position" role="listbox" :aria-label="label"><div v-for="(option,index) in options" :id="`${id}-${index}`" :key="option.value" class="select-option" :class="{highlight:active===index,chosen:option.value===modelValue}" role="option" :aria-selected="option.value===modelValue" @pointerenter="active=index" @mousedown.prevent @click="choose(index)">{{ option.label }}</div></div></Teleport></div>
</template>
