# Fork Constructo AI d'Open CAD Studio

Ce dépôt est une **œuvre dérivée d'Open CAD Studio**, distribué sous licence
**GNU General Public License version 3** (voir `LICENSE`, inchangé).

| | |
|---|---|
| Amont | https://github.com/HakanSeven12/OpenCADStudio |
| Auteur amont | HakanSeven12 et les contributeurs d'Open CAD Studio |
| Licence | GPL-3.0-or-later — **inchangée, et elle ne peut pas l'être** |
| Révision de base du fork | `28e53047` (v2026.37 + 369 commits), clonée le 2026-09-19 |
| But du fork | Brancher le moteur CAD (DWG/DXF) sur l'ERP Constructo AI : ouvrir et enregistrer les dessins dans les dossiers de l'ERP plutôt que sur le poste |

## Ce que la GPL impose ici, en clair

La GPL-3.0 **n'est pas l'AGPL**. Elle se déclenche à la **distribution**, pas à
l'usage.

- **Faire tourner ce code sur nos serveurs** (conversion, headless) ne crée
  aucune obligation de publication.
- **Servir le bundle WebAssembly au navigateur d'un client EST une
  distribution.** Dès le premier client servi, nous devons offrir à ce client
  le code source correspondant — celui du bundle qu'il a reçu, nos
  modifications comprises — sous GPL-3.0.

Conséquences pratiques, à tenir :

1. ✅ **FAIT le 2026-09-19** : ce dépôt est **public**, le jour même de la mise
   en service. Il l'était resté privé tant que rien n'était servi ; servir le
   bundle à un navigateur est une distribution, donc les sources
   correspondantes devaient devenir accessibles. C'est le cas.
2. Le module `/cad` de l'ERP doit porter, visible depuis l'interface, un lien
   vers ces sources et la mention de la licence.
3. Les modifications que nous écrivons ici sont **GPL-3.0 elles aussi**. Elles
   ne peuvent pas être reversées dans le code propriétaire de l'ERP.

## La frontière, et pourquoi elle est nette

Le code GPL vit **dans ce dépôt seulement**. Il n'entre pas dans le monorepo
`Constructo_AI_Prod`, qui reste propriétaire.

Ce qui traverse la frontière est un **artefact compilé** servi sous `/cad/` et
des **appels HTTP** vers l'API de l'ERP. Deux programmes qui se parlent par le
réseau ne forment pas une œuvre unique : l'ERP ne devient pas GPL, et ce fork
ne devient pas propriétaire.

➜ **Ne jamais copier de code de ce dépôt vers `Constructo_AI_Prod`, ni
l'inverse.** C'est ce geste-là, et lui seul, qui ferait basculer l'ERP sous
GPL.

## Travailler avec l'amont

```bash
git remote -v
# origin    https://github.com/ConstructoAI/constructo-cad.git   (le fork, PUBLIC)
# upstream  https://github.com/HakanSeven12/OpenCADStudio.git    (l'amont)

git fetch upstream && git rebase upstream/main
```

Les workflows hérités qui agiraient seuls (`weekly-release`, `issue-welcome`,
`pages`, `release`) sont désactivés **côté GitHub**, pas dans les fichiers :
un fichier modifié ferait conflit à chaque rebase sur l'amont. `web-check`
reste actif — c'est lui qui dit si une modification casse la cible web.
