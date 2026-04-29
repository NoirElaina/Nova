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
const activePreview = reactive<Record<string, string>>({});
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

const dialogTitle = computed(() => {
  const firstHeader = currentQuestion.value?.header ?? questions.value[0]?.header ?? '';
  return firstHeader.includes('权限') ? '请确认权限操作' : '我需要你确认几个关键选项';
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
    Object.keys(activePreview).forEach((key) => delete activePreview[key]);
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

  activePreview[key] = option.preview ?? '';
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

  const freeformParts = questions.value
    .map((q, index) => {
      const value = (freeformAnswers[questionStateKey(q, index)] ?? '').trim();
      return value ? `${q.header}: ${value}` : '';
    })
    .filter(Boolean);

  return {
    answers,
    answerItems,
    freeform: freeformParts.join('\n') || undefined,
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
  <div v-if="request" class="ask-shell">
    <div class="ask-card">
      <div class="ask-header">
        <div>
          <div class="ask-title">{{ dialogTitle }}</div>
          <div v-if="request.context" class="ask-context">{{ request.context }}</div>
        </div>
        <Button variant="ghost" size="icon-sm" class="ask-close" title="关闭" @click="emit('skip')">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <path d="M6 6l12 12M18 6L6 18" stroke-linecap="round" />
          </svg>
        </Button>
      </div>

      <div v-if="currentQuestion" class="ask-question-list">
        <section class="ask-question-card">
          <div class="ask-question-header">
            <div class="ask-question-meta">
              <span class="ask-chip">{{ currentQuestion.header }}</span>
              <span class="ask-mode">{{ currentQuestion.multi_select ? '可多选' : '单选' }}</span>
            </div>
            <span v-if="progressText" class="ask-progress">{{ progressText }}</span>
          </div>
          <div class="ask-question-title">{{ currentQuestion.question }}</div>

          <div class="ask-options">
            <Button
              variant="ghost"
              v-for="(option, index) in currentQuestion.options"
              :key="`${currentQuestionKey}-${option.label}-${index}`"
              class="ask-option"
              :class="{ 'is-selected': isSelected(currentQuestion, currentIndex, option) }"
              @click="toggleOption(currentQuestion, currentIndex, option)"
            >
              <span class="ask-index">{{ index + 1 }}</span>
              <span class="ask-option-body">
                <span class="ask-label">{{ option.label }}</span>
                <span class="ask-description">{{ option.description }}</span>
              </span>
            </Button>
          </div>

          <div
            v-if="activePreview[currentQuestionKey]"
            class="ask-preview"
          >
            <div class="ask-preview-title">Preview</div>
            <pre class="ask-preview-body">{{ activePreview[currentQuestionKey] }}</pre>
          </div>

        </section>
      </div>

      <div v-if="request.allow_freeform !== false" class="ask-freeform">
        <div class="ask-freeform-title">其他补充</div>
        <Textarea
          v-model="freeform"
          class="ask-freeform-input"
          rows="3"
          placeholder="如果上面的选项还不够准确，可以在这里补充说明"
        />
      </div>

      <div class="ask-actions">
        <Button
          variant="outline"
          size="sm"
          v-if="questions.length > 1"
          class="ask-back"
          :disabled="currentIndex === 0"
          @click="goPrevious"
        >
          上一步
        </Button>
        <Button variant="outline" size="sm" class="ask-skip" @click="emit('skip')">跳过</Button>
        <Button size="sm" class="ask-submit" :disabled="!canSubmit" @click="submitAnswers">
          {{ isLastQuestion ? '确认' : '下一步' }}
        </Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ask-shell {
  width: 100%;
  box-sizing: border-box;
}

.ask-card {
  width: 100%;
  max-width: 760px;
  margin: 0 auto;
  box-sizing: border-box;
  border: 1px solid #ddd7ca;
  border-radius: 20px;
  background: #fffdfa;
  padding: 14px;
  box-shadow: 0 14px 40px rgba(45, 34, 18, 0.1);
}

.ask-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 2px 2px 12px;
}

.ask-title {
  color: #262117;
  font-size: 16px;
  font-weight: 600;
  line-height: 1.4;
}

.ask-context {
  margin-top: 6px;
  color: #847b6d;
  font-size: 12px;
  line-height: 1.5;
}

.ask-close {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: #746d60;
}

.ask-close:hover {
  background: #f3eee4;
}

.ask-question-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ask-question-card {
  border: 1px solid #ece6da;
  border-radius: 16px;
  padding: 12px;
  background: #fffefb;
}

.ask-question-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 8px;
}

.ask-question-meta {
  display: flex;
  align-items: center;
  gap: 8px;
}

.ask-chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 24px;
  padding: 0 10px;
  border-radius: 999px;
  background: #f2ede4;
  color: #6d6557;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.03em;
}

.ask-mode {
  color: #a19686;
  font-size: 11px;
}

.ask-progress {
  color: #8b816f;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

.ask-question-title {
  margin-bottom: 10px;
  color: #262117;
  font-size: 14px;
  line-height: 1.5;
}

.ask-options {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ask-option {
  width: 100%;
  min-height: 74px;
  height: auto !important;
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid #ece6da;
  border-radius: 14px;
  background: transparent;
  text-align: left;
  white-space: normal;
}

.ask-option:hover {
  background: #faf6ed;
}

.ask-option.is-selected {
  background: #f6f1e7;
  border-color: #d9cfbc;
}

.ask-index {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  border-radius: 10px;
  background: #ece7dd;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #6a6357;
  font-size: 13px;
}

.ask-option-body {
  min-width: 0;
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 6px;
}

.ask-label {
  color: #262117;
  font-size: 14px;
  font-weight: 500;
  line-height: 1.35;
}

.ask-description {
  color: #857d6f;
  font-size: 12px;
  line-height: 1.5;
}

.ask-preview {
  margin-top: 10px;
  padding: 10px 12px;
  border-radius: 14px;
  background: #f7f3ea;
  border: 1px solid #ebe3d4;
}

.ask-preview-title {
  margin-bottom: 6px;
  color: #766d5f;
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.ask-preview-body {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  color: #4f473b;
  font-size: 12px;
  line-height: 1.6;
  font-family: 'SF Mono', 'Fira Code', monospace;
}

.ask-freeform-input {
  width: 100%;
  margin-top: 10px;
  border: 1px solid #e6dece;
  border-radius: 12px;
  background: #fffdfa;
  padding: 10px 12px;
  resize: vertical;
  box-sizing: border-box;
  outline: none;
  color: #262117;
  font-size: 13px;
  line-height: 1.5;
}

.ask-freeform {
  margin-top: 12px;
}

.ask-freeform-title {
  color: #6a6357;
  font-size: 12px;
  font-weight: 600;
}

.ask-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 14px;
}

.ask-back,
.ask-skip,
.ask-submit {
  flex-shrink: 0;
  border-radius: 10px;
  padding: 8px 14px;
  font-size: 13px;
}

.ask-back,
.ask-skip {
  border: 1px solid #d4ccbf;
  background: #fffdfa;
  color: #262117;
}

.ask-submit {
  border: 1px solid #d38f6f;
  background: #da7756;
  color: white;
}

.ask-back:disabled,
.ask-submit:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
