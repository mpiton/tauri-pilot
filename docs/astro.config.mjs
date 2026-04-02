// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	site: 'https://mpiton.github.io',
	base: '/tauri-pilot/',
	integrations: [
		starlight({
			title: 'tauri-pilot',
			logo: {
				src: './src/assets/logo.jpeg',
			},
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/mpiton/tauri-pilot' },
			],
			customCss: ['./src/styles/custom.css'],
			sidebar: [
				{
					label: 'Start Here',
					items: [
						{ slug: 'getting-started' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ slug: 'guides/plugin-setup' },
						{ slug: 'guides/architecture' },
						{ slug: 'guides/ai-agents' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ slug: 'reference/cli' },
					],
				},
				{
					label: 'Community',
					items: [
						{ slug: 'contributing' },
					],
				},
			],
		}),
	],
});
