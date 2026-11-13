import { createDomStorybookConfig } from '@overeng/utils-storybook/config'

/** Plain DOM Storybook: the stories only frame the TrayGallery PNGs in `../renders/`. */
export default createDomStorybookConfig({
  stories: ['../src/**/*.stories.@(ts|tsx)'],
})
