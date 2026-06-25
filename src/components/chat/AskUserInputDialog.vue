<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import type {
  AskUserAnswerSubmission,
  AskUserOption,
  AskUserQuestionItem,
  NeedsUserInputPayload,
} from '@/lib/chat-types';

const props = defineProps<{
  request: NeedsUserInputPayload | null;
}>();

const emit = defineEmits<{
  (e: 'submit', value: AskUserAnswerSubmission): void;
  (e: 'skip'): void;
}>();

const selectedAnswers = reactive<Record<string, string[]>>({});
const freeformAnswers = reactive<Record<string, string>>({});
const freeform = ref('');
const currentIndex = ref(0);

const questions = computed(() => props.request?.questions ?? []);
const currentQuestion = computed(() => questions.value[currentIndex.value] ?? null);
const currentQuestionKey = computed(() => {
  const question = currentQuestion.value;
  return question ? questionStateKey(question, currentIndex.value) : '';
});
const isLastQuestion = computed(() => currentIndex.value >= questions.value.length - 1);
const progressText = computed(() => {
  if (questions.value.length <= 1) return '';
  return `${currentIndex.value + 1} / ${questions.value.length}`;
});

function optionAnswerValue(option: AskUserOption): string {
  return (option.value?.trim() || option.label).trim();
}

function questionStateKey(question: AskUserQuestionItem, index: number): string {
  const explicitId = question.id?.trim();
  return explicitId || `question-${index}`;
}

function questionAnswerLabel(question: AskUserQuestionItem, index: number): string {
  const base = question.question.trim() || question.header.trim() || `问题 ${index + 1}`;
  return base;
}

function saveCurrentFreeform() {
  const key = currentQuestionKey.value;
  if (key) {
    freeformAnswers[key] = freeform.value;
  }
}

function restoreFreeform(index: number) {
  const question = questions.value[index];
  const key = question ? questionStateKey(question, index) : '';
  freeform.value = key ? (freeformAnswers[key] ?? '') : '';
}

watch(
  () => props.request,
  () => {
    Object.keys(selectedAnswers).forEach((key) => delete selectedAnswers[key]);
    Object.keys(freeformAnswers).forEach((key) => delete freeformAnswers[key]);
    freeform.value = '';
    currentIndex.value = 0;
  },
  { immediate: true },
);

function toggleOption(question: AskUserQuestionItem, index: number, option: AskUserOption) {
  const key = questionStateKey(question, index);
  const current = selectedAnswers[key] ?? [];
  const target = optionAnswerValue(option);

  if (question.multi_select) {
    selectedAnswers[key] = current.includes(target)
      ? current.filter((item) => item !== target)
      : [...current, target];
  } else {
    selectedAnswers[key] = [target];
  }
}

function isSelected(question: AskUserQuestionItem, index: number, option: AskUserOption) {
  return (selectedAnswers[questionStateKey(question, index)] ?? []).includes(optionAnswerValue(option));
}

const canSubmit = computed(() => {
  const key = currentQuestionKey.value;
  if (!key) return false;
  const answers = selectedAnswers[key] ?? [];
  return answers.length > 0 || freeform.value.trim().length > 0;
});

function buildSubmission(): AskUserAnswerSubmission {
  saveCurrentFreeform();
  const answers: Record<string, string | string[]> = {};
  const answerItems: NonNullable<AskUserAnswerSubmission['answerItems']> = [];
  const answerLabelCounts = new Map<string, number>();

  for (const [index, question] of questions.value.entries()) {
    const stateKey = questionStateKey(question, index);
    const values = selectedAnswers[stateKey] ?? [];
    const qFreeform = (freeformAnswers[stateKey] ?? '').trim();
    const answer =
      question.multi_select
        ? values.length > 0 ? values : (qFreeform ? [qFreeform] : [])
        : values[0] ?? qFreeform ?? '';
    const baseLabel = questionAnswerLabel(question, index);
    const count = (answerLabelCounts.get(baseLabel) ?? 0) + 1;
    answerLabelCounts.set(baseLabel, count);
    const answerLabel = count === 1 ? baseLabel : `${baseLabel} (${count})`;

    answers[answerLabel] = answer;
    answerItems.push({
      key: stateKey,
      question: question.question,
      header: question.header,
      answer,
    });
  }

  return {
    answers,
    answerItems,
  };
}

function goNext() {
  if (!canSubmit.value || isLastQuestion.value) return;
  saveCurrentFreeform();
  currentIndex.value += 1;
  restoreFreeform(currentIndex.value);
}

function goPrevious() {
  if (currentIndex.value <= 0) return;
  saveCurrentFreeform();
  currentIndex.value -= 1;
  restoreFreeform(currentIndex.value);
}

function submitAnswers() {
  if (!currentQuestion.value) return;
  if (!isLastQuestion.value) {
    goNext();
    return;
  }

  if (!canSubmit.value) return;
  emit('submit', buildSubmission());
}
</script>

<template>
  <div v-if="request" class="ask-box">
    <div class="ask-top">
      <div class="ask-meta">
        <span v-if="currentQuestion" class="ask-chip">{{ currentQuestion.header }}</span>
        <span v-if="progressText" class="ask-progress">{{ progressText }}</span>
      </div>
      <button type="button" class="ask-skip-btn" title="跳过" @click="emit('skip')">跳过</button>
    </div>

    <div v-if="request.context" class="ask-context">{{ request.context }}</div>

    <div v-if="currentQuestion" class="ask-body">
      <div class="ask-question-text">{{ currentQuestion.question }}</div>

      <div class="ask-options">
        <button
          type="button"
          v-for="(option, index) in currentQuestion.options"
          :key="`${currentQuestionKey}-${option.label}-${index}`"
          class="ask-option"
          :class="{ 'is-selected': isSelected(currentQuestion, currentIndex, option) }"
          @click="toggleOption(currentQuestion, currentIndex, option)"
        >
          <span class="ask-option-label">{{ option.label }}</span>
          <span v-if="option.description" class="ask-option-desc">{{ option.description }}</span>
        </button>
      </div>

      <div v-if="request.allow_freeform !== false" class="ask-freeform-row">
        <Textarea
          v-model="freeform"
          class="ask-freeform-input"
          rows="2"
          :placeholder="currentQuestion.options.length > 0 ? '或直接输入你的回答' : '输入你的回答'"
        />
      </div>
    </div>

    <div class="ask-bottom">
      <Button
        v-if="questions.length > 1"
        variant="ghost"
        size="sm"
        class="ask-nav-btn"
        :disabled="currentIndex === 0"
        @click="goPrevious"
      >
        上一步
      </Button>
      <div class="flex-1" />
      <Button size="sm" class="ask-submit-btn" :disabled="!canSubmit" @click="submitAnswers">
        {{ isLastQuestion ? '确认' : '下一步' }}
      </Button>
    </div>
  </div>
</template>

<style scoped>
.ask-box {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid #e5e7eb;
  border-radius: 14px;
  background: #ffffff;
  padding: 10px 12px;
}

.dark .ask-box {
  border-color: #3f3f46;
  background: #1a1a1a;
}

.ask-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 6px;
}

.ask-meta {
  display: flex;
  align-items: center;
  gap: 6px;
}

.ask-chip {
  display: inline-flex;
  align-items: center;
  padding: 1px 8px;
  border-radius: 999px;
  background: #f1f5f9;
  color: #475569;
  font-size: 11px;
  font-weight: 600;
}

.dark .ask-chip {
  background: #27272a;
  color: #a1a1aa;
}

.ask-progress {
  color: #94a3b8;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.ask-skip-btn {
  border: 0;
  background: transparent;
  color: #94a3b8;
  font-size: 12px;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 6px;
  transition: background 120ms ease, color 120ms ease;
}

.ask-skip-btn:hover {
  background: #f1f5f9;
  color: #64748b;
}

.dark .ask-skip-btn:hover {
  background: #27272a;
  color: #d4d4d8;
}

.ask-context {
  margin-bottom: 8px;
  padding: 8px 10px;
  border-radius: 8px;
  background: #f8fafc;
  border: 1px solid #e2e8f0;
  color: #0f172a;
  font-size: 12.5px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}

.dark .ask-context {
  background: #111827;
  border-color: #1f2937;
  color: #e2e8f0;
}

.ask-body {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.ask-question-text {
  color: #111827;
  font-size: 13px;
  line-height: 1.5;
}

.dark .ask-question-text {
  color: #ececec;
}

.ask-options {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.ask-option {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 10px 14px;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background 120ms ease, border-color 120ms ease;
}

.ask-option:hover {
  background: #f8fafc;
  border-color: #cbd5e1;
}

.ask-option.is-selected {
  background: #eff6ff;
  border-color: #93c5fd;
}

.dark .ask-option {
  border-color: #3f3f46;
}

.dark .ask-option:hover {
  background: #27272a;
  border-color: #52525b;
}

.dark .ask-option.is-selected {
  background: rgba(59, 130, 246, 0.12);
  border-color: #3b82f6;
}

.ask-option-label {
  color: #111827;
  font-size: 13px;
  font-weight: 500;
  line-height: 1.4;
}

.dark .ask-option-label {
  color: #ececec;
}

.ask-option-desc {
  color: #64748b;
  font-size: 11px;
  line-height: 1.4;
  margin-left: auto;
  text-align: right;
}

.dark .ask-option-desc {
  color: #a1a1aa;
}

.ask-freeform-row {
  margin-top: 2px;
}

.ask-freeform-input {
  width: 100%;
  border: 1px solid #e5e7eb;
  border-radius: 10px;
  background: #ffffff;
  padding: 7px 10px;
  resize: none;
  box-sizing: border-box;
  outline: none;
  color: #111827;
  font-size: 13px;
  line-height: 1.5;
}

.dark .ask-freeform-input {
  border-color: #3f3f46;
  background: #111827;
  color: #ececec;
}

.ask-bottom {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}

.ask-nav-btn {
  color: #64748b;
}

.ask-submit-btn {
  border-radius: 10px;
  background: #111827;
  color: white;
  font-size: 13px;
  padding: 6px 16px;
}

.ask-submit-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.dark .ask-submit-btn {
  background: #ececec;
  color: #111827;
}
</style>
