import type { ChangelogEntrySource } from '../src/changelog/contract.ts'
export default {
  title: 'Wait details distinguish replies from running jobs',
  summary: 'Recorded wait details name the sender who replied and list completed jobs as arrivals, while active jobs stay labelled Still running. Tool details use the existing inspector and the SDK’s current block decoding.',
  kind: 'fix',
} satisfies ChangelogEntrySource
