/* Salvage — wording, in every language the window speaks.
 *
 * The architecture was built for this from the start: the domain returns
 * classifications and numbers, never sentences, and the bridge returns lookup
 * keys. So every sentence a user reads is in this file, and nothing outside it
 * had to learn a second language.
 *
 * Numbers are formatted here too, and that is not decoration. A thousands
 * separator is a period in Brazil and a comma in the United States; getting it
 * from `Intl` per locale is the only way "124.735.488" and "124,735,488" both
 * come out right. It is also why the bridge stopped sending pre-formatted
 * labels: a string it had already rendered could not be re-rendered here.
 *
 * A key with no entry falls through to whatever it was given. That is
 * deliberate: a rare technical failure arriving from a lower layer is shown as
 * its own text rather than replaced by a missing-translation placeholder.
 */

const I18N = (() => {
  const DICT = {
    /* ─────────────────────────────────────────────── Portuguese (Brazil) */
    "pt-BR": {
      "lang.name": "Português (BR)",

      "app.tagline": "Inspeção de setores e isolamento de área defeituosa",
      "splash.loading": "Carregando…",

      "ui.refresh": "Atualizar dispositivos",
      "ui.language": "Idioma",
      "ui.device": "Dispositivo",
      "ui.selectCard": "Selecione o cartão",
      "ui.searching": "Procurando…",
      "ui.choose": "Selecione…",
      "ui.blockedSuffix": " — bloqueado",
      "ui.listFailed": "Falha ao listar",
      "ui.noDevice": "Nenhum dispositivo",
      "ui.noCardDetected":
        "Nenhum cartão detectado. Insira o microSD e clique em <strong>Atualizar dispositivos</strong>.",
      "ui.inspection": "Inspeção",
      "ui.inspectionHint":
        "Grava um padrão em todos os setores e confere um por um, do fim para o começo. " +
        "Pinta de <strong>verde</strong> o que voltar intacto e de <strong>vermelho</strong> " +
        "o que falhar. É a única forma de aprovar área para receber dados — e apaga todo o " +
        "conteúdo do cartão.",
      "ui.inspect": "Inspecionar cartão",
      "ui.cancel": "Cancelar",
      "ui.result": "Resultado",
      "ui.resultEmpty": "O resultado aparece quando a inspeção termina.",
      "ui.evidence": "Evidência da medição",
      "ui.evidenceCount": "{n} itens",
      "ui.isolation": "Isolamento",
      "ui.isolationEmpty": "Depois da inspeção é possível separar a área aprovada da defeituosa.",
      "ui.filesystem": "Sistema de arquivos da área visível",
      "ui.applyPlan": "Aplicar layout",
      "ui.plansLabel": "Layouts disponíveis",
      "ui.failure": "Falha na interface: {msg}",
      "ui.thecard": "o cartão",
      "ui.sectorStates": "Setores por estado",

      "spec.capacity": "Capacidade",
      "spec.bus": "Barramento",
      "spec.sectors": "Setores",
      "spec.volumes": "Volumes",
      "spec.of": "de {total}",
      "spec.none": "nenhum",
      "spec.removable": "removível",
      "spec.fixed": "fixo",

      "fs.exfat": "exFAT — arquivos acima de 4 GB",
      "fs.fat32": "FAT32 — compatibilidade máxima",

      "verdict.allowed": "Liberado para inspeção",
      "verdict.needs_confirmation": "Exige confirmação",
      "verdict.blocked": "Bloqueado pelas travas de segurança",
      "verdict.noNotes": "Nenhuma ressalva.",
      "verdict.usable": "aproveitáveis",
      "verdict.split": "{good}% íntegro · {bad}% defeituoso",
      "verdict.splitUnverified": " · {rest}% não conferido",
      "verdict.splitTotal": " — de {total}",

      "prior.title": "Este cartão já foi isolado pelo Salvage.",
      "prior.body":
        "A inspeção vai tratar apenas dos <strong>{remaining}</strong> que sobraram. " +
        "Os {fenced} já cercados ficam de fora: foram condenados numa passagem anterior " +
        "e não há o que reaprender neles.",
      "prior.scattered":
        "A área em uso está dividida em {count} partições, e a inspeção cobre o intervalo " +
        "que vai da primeira à última — inclusive a quarentena entre elas. Deixar uma delas " +
        "de fora é que não serve: ela ficaria montada sem nunca ter sido conferida.",

      "stage.selectFirst": "Selecione um cartão e inicie a inspeção",
      "stage.blocked": "Este dispositivo está bloqueado pelas travas de segurança.",
      "stage.ready": "{name} · {size} — pronto para inspeção.",

      "phase.waiting": "Aguardando",
      "phase.idle": "Aguardando",
      "phase.writing": "Gravando padrão — do fim para o início",
      "phase.verifying": "Relendo e comparando — do início para o fim",
      "phase.refining": "Isolando os setores responsáveis por bissecção",
      "phase.done": "Inspeção concluída",
      "progress.of": "{done} de {total}",
      "progress.defects": " · {n} setores com defeito",
      "progress.rate": " · {speed}/s · restam {time}",
      "progress.inspected": "{size} inspecionados",
      "dur.seconds": "{n} s",
      "dur.minutes": "{n} min",
      "dur.hours": "{h} h {m} min",

      "state.0": "Não conferido",
      "state.1": "Íntegro",
      "state.2": "Erro de leitura",
      "state.3": "Erro de gravação",
      "state.4": "Devolveu conteúdo alterado",
      "state.5": "Devolveu dado de outro endereço",
      "state.6": "Isolado — não recebe dados",

      "scenario.pristine": "Nenhum defeito encontrado",
      "scenario.counterfeit_capacity": "Capacidade falsificada",
      "scenario.exhausted_spare": "Blocos reserva esgotados",
      "scenario.actively_degrading": "Degradação ativa",
      "scenario.indeterminate": "Defeitos encontrados, estabilidade desconhecida",
      "scenario.not_proven": "Inspeção incompleta",

      "mechanism.pristine": "Todos os setores foram gravados e relidos idênticos.",
      "mechanism.counterfeit_capacity":
        "O cartão anuncia mais memória do que possui e serve endereços altos com células " +
        "baixas. A fronteira vem do firmware e não se move com o uso.",
      "mechanism.exhausted_spare":
        "O controlador gastou os blocos reserva que usava para substituir células ruins. " +
        "Sem reserva, os defeitos congelaram em endereços fixos.",
      "mechanism.actively_degrading":
        "Setores aprovados na primeira passagem falharam na segunda. As células estão " +
        "morrendo agora, durante a própria inspeção.",
      "mechanism.indeterminate":
        "Os defeitos são reais, mas uma passagem só não distingue defeito estável de " +
        "deterioração em curso.",
      "mechanism.not_proven":
        "A inspeção não cobriu o cartão inteiro. Sobre os setores que ela não alcançou, " +
        "nada é conhecido.",

      "assurance.high": "Confiança alta",
      "assurance.moderate": "Confiança moderada",
      "assurance.low": "Confiança baixa",
      "assurance.none": "Sem base para confiar",

      "statement.high":
        "A fronteira entre a área boa e a ruim é imposta pelo firmware e não se move com o " +
        "uso. O que foi aprovado continuará aprovado.",
      "statement.moderate":
        "Os defeitos aparentam estar estáveis, mas o cartão está no fim da vida útil. Use " +
        "para dados que você já tem copiados em outro lugar.",
      "statement.low":
        "O isolamento cobre os defeitos já conhecidos. Novos podem surgir em endereços hoje " +
        "aprovados. Nada insubstituível deve ficar aqui.",
      "statement.none":
        "A medição não sustenta conclusão alguma: ou o cartão piorou durante a própria " +
        "inspeção, ou ela não cobriu o cartão inteiro.",

      "detail.largestRun": "Maior trecho contínuo aprovado: <strong>{size}</strong>",
      "detail.announced_capacity": "Capacidade anunciada: {size}",
      "detail.real_capacity": "Capacidade real estimada: {size}",
      "detail.alias_evidence": "Endereços que devolveram dado de outro endereço: {n}",
      "detail.defect_regions": "Regiões defeituosas distintas: {n}",
      "detail.second_pass_identical": "Segunda passagem: os mesmos defeitos, nos mesmos lugares",
      "detail.newly_failed_sectors": "Setores aprovados na 1ª passagem que falharam na 2ª: {size}",
      "detail.new_regions": "Regiões que surgiram entre as duas passagens: {n}",
      "detail.unverified_area": "Área não conferida: {size}",
      "detail.one_pass_only": "Passagens executadas: 1 — insuficiente para julgar estabilidade",

      "strategy.largest_contiguous": "Faixa contígua única",
      "strategy.maximum_space": "Várias faixas separadas",
      "strategy.conservative": "Faixa única, margem ampliada",
      "strategy.spliced_fat32": "Volume emendado (FAT32)",

      "cost.largest_contiguous": "1 unidade · com margem",
      "cost.maximum_space": "várias unidades · com margem",
      "cost.conservative": "1 unidade · margem 4×",
      "cost.spliced_fat32": "1 unidade · sem margem · arquivo ≤ 4 GB",

      "note.largest_contiguous":
        "Uma unidade sobre o maior trecho contínuo aprovado. O resto do cartão vai para " +
        "partições que o Windows não monta. Mantém a margem de segurança em volta de cada " +
        "defeito.",
      "note.maximum_space":
        "Uma unidade para cada trecho aprovado grande o bastante. Aproveita mais espaço que " +
        "a faixa única, ao custo de várias letras de unidade. Mantém a margem de segurança.",
      "note.conservative":
        "Igual à faixa contígua única, com margem quatro vezes maior em volta dos defeitos. " +
        "Menos espaço, mais distância entre seus dados e a célula morta.",
      "note.spliced_fat32":
        "Uma unidade só, cujo espaço livre é a soma de todos os trechos aprovados. Cada " +
        "cluster que encosta em setor defeituoso é marcado na tabela do sistema de arquivos " +
        "e nunca é entregue a um arquivo. Sem margem de segurança: o vizinho de uma célula " +
        "morta continua disponível. Arquivo individual limitado a 4 GB.",

      "plan.visible": "visível",
      "plan.hidden": "oculta",
      "plan.part": "{label} ({kind}, tipo {type})",
      "plan.sacrificed":
        "{size} de área aprovada ficam de fora por margem de segurança e alinhamento de bloco.",
      "plan.fencedToast": "{size} de área aprovada ficaram cercados pela margem de segurança.",
      "legend.written": "Gravado, aguardando conferência",
      "plan.cannot": "Não há como particionar este cartão.",
      "refusal.lead":
        "A área aprovada existe, mas está fragmentada. Uma partição ocupa um " +
        "intervalo contínuo de setores, e cada mecanismo tem um tamanho mínimo " +
        "que nenhum dos pedaços alcança.",
      "refusal.approved": "Aprovado no total",
      "refusal.inPieces": "em {n} pedaços separados",
      "refusal.largest": "Maior pedaço contínuo",
      "refusal.fencedNeeds": "Partição cercada precisa de",
      "refusal.splicedNeeds": "Volume emendado FAT32 precisa de",
      "refusal.why":
        "A partição cercada precisa da margem de segurança dos dois lados mais " +
        "o tamanho mínimo. O volume emendado precisa desse espaço contínuo e " +
        "íntegro logo no começo, só para a tabela de alocação e o diretório " +
        "raiz — não há onde colocá-los.",
      "plan.cannotWhy":
        "Uma partição só pode ser criada sobre área contígua verificada. Neste cartão não " +
        "sobrou nenhuma grande o bastante.",
      "plan.none.pristine": "Nenhum defeito foi encontrado: não há área a isolar neste cartão.",
      "plan.none.not_proven":
        "A inspeção não cobriu o cartão inteiro. Rode-a até o fim para que haja área " +
        "aprovada sobre a qual planejar.",
      "plan.none.degrading":
        "O cartão perdeu setores durante a própria inspeção. Nenhum layout de partição " +
        "protege contra defeitos que ainda não existem.",

      "modal.confirmTitle": "Confirmar operação destrutiva",
      "modal.typeName": "Para confirmar, digite o nome do dispositivo:",
      "modal.confirm": "Confirmar",
      "modal.cancel": "Cancelar",
      "modal.close": "Fechar",

      "scan.confirmTitle": "Confirmar inspeção destrutiva",
      "scan.scopeAll": "em todos os setores de {name} ({size})",
      "scan.scopePrior": "nos {size} que a última inspeção deixou em uso em {name}",
      "scan.confirmBody":
        "<p class=\"destructive\">A inspeção grava {scope}. Todo o conteúdo será perdido.</p>" +
        "<p>Cada setor é gravado e depois relido. Só o que voltar idêntico é aprovado — é " +
        "assim que capacidade falsificada e corrupção silenciosa aparecem, e nenhuma das " +
        "duas se revela de outro jeito.</p>",
      "scan.warningTitle": "Atenção:",
      "scan.cancelled": "Inspeção cancelada.",
      "scan.done": "Inspeção concluída.",
      "scan.watchdog":
        "A inspeção foi iniciada mas não reportou andamento em {n}s. Consulte o arquivo de " +
        "diagnóstico indicado no rodapé.",

      "apply.confirmTitle": "Confirmar reparticionamento",
      "apply.erases":
        "Esta operação apaga tudo o que existe em {name} e grava uma nova tabela de partições.",
      "apply.layoutIs": "Layout <strong>{name}</strong>:",
      "apply.dataParts":
        "<strong>{n}</strong> partição(ões) visível(eis), somando <strong>{size}</strong>, " +
        "sobre área aprovada setor a setor.",
      "apply.hiddenParts":
        "<strong>{n}</strong> partição(ões) oculta(s) do tipo 0xDA, que o Windows não monta " +
        "nem exibe.",
      "apply.caveat":
        "O isolamento reduz o risco, mas não transforma um cartão defeituoso em confiável. " +
        "Mantenha cópia de qualquer dado insubstituível.",
      "apply.applying": "Aplicando layout… não remova o cartão.",
      "apply.doneTitle": "Layout aplicado",
      "apply.where": "A área confiável está disponível em <strong>{letter}:</strong>",
      "apply.done": "Reparticionamento concluído.",

      "close.title": "Inspeção em andamento",
      "close.body":
        "<p class=\"destructive\">Há uma inspeção em andamento em {name}.</p>" +
        "<p>Fechar agora interrompe a passagem. O que já foi gravado no cartão não volta " +
        "atrás e nada fica aprovado: a inspeção só aprova área que ela mesma releu " +
        "inteira.</p>" +
        "<p>O programa aguarda a inspeção largar o cartão antes de encerrar, para que o " +
        "Windows volte a enxergá-lo.</p>",
      "close.stop": "Parar e fechar",
      "close.keep": "Continuar a inspeção",
      "close.leaving": "Encerrando: aguardando a inspeção largar o cartão…",

      "step.volumes_dismounted": "Volumes existentes bloqueados e desmontados.",
      "step.volume_warning": "Aviso: {detail}",
      "step.table_changed":
        "Entrada {slot}: {action} — tipo {type}, setores {from} a {to} ({n} setores)",
      "step.table_changed.added": "criada",
      "step.table_changed.removed": "removida",
      "step.partition_head_wiped":
        "Início da partição '{label}' zerado, para que nenhum sistema de arquivos antigo " +
        "seja reconhecido sobre o novo layout.",
      "step.table_written": "Nova tabela de partições gravada no setor 0.",
      "step.system_notified": "Windows avisado para reler o layout do disco.",
      "step.volume_mounted": "Partição de dados montada em {letter}:",
      "step.formatted": "{letter}: formatada como {filesystem}.",
      "step.cluster_map_written":
        "Volume FAT32 gravado com {n} clusters marcados como defeituosos na tabela de " +
        "alocação. Nenhum driver entrega esses clusters a um arquivo.",
      "step.mount_timed_out":
        "A partição foi criada, mas o Windows não a montou a tempo. Remova e reinsira o " +
        "cartão para concluir a formatação.",

      "block.hosts_operating_system":
        "Este disco contém o Windows (volumes {volumes}). Gravar nele deixa a máquina sem " +
        "inicializar.",
      "block.hosts_operating_system.system": "de sistema",
      "block.internal_bus":
        "Conectado por {bus}, barramento usado apenas por disco interno. Nenhum leitor de " +
        "cartão se apresenta assim.",
      "block.exceeds_addressable_capacity":
        "Capacidade de {capacity}, acima do limite de {limit} que o programa endereça.",
      "block.zero_capacity":
        "O dispositivo responde com capacidade zero: não há mídia inserida, ou ela parou de " +
        "responder.",

      "warn.not_declared_removable":
        "O Windows não marca esta mídia como removível. Adaptadores USB de microSD costumam " +
        "se apresentar assim, mas confirme o alvo.",
      "warn.has_mounted_volumes": "Os volumes {volumes} estão montados e serão perdidos.",
      "warn.unusually_large":
        "Capacidade de {capacity} é alta para um cartão. Confirme que o alvo é o que você " +
        "pretende apagar.",
      "warn.unknown_bus":
        "Não foi possível identificar por qual barramento o dispositivo está conectado, " +
        "então as verificações que dependem disso não se aplicaram.",

      "footer.diagnostics": "Diagnóstico:",
      "footer.by": "por ",
      "footer.openLog": "Abrir o arquivo de diagnóstico",

      "err.state": "O estado interno do programa foi corrompido. Feche e abra de novo.",
      "err.enumerate":
        "Não foi possível listar os dispositivos. O programa precisa de privilégio de " +
        "Administrador para abrir discos.",
      "err.device_gone": "Dispositivo não encontrado; atualize a lista.",
      "err.scan_running": "Já existe uma inspeção em andamento.",
      "err.no_device": "Selecione um dispositivo antes.",
      "err.no_map": "Inspecione o cartão antes de planejar.",
      "err.no_plan": "Esse layout não existe mais; calcule os layouts de novo.",
      "err.plan_rejected":
        "O layout foi rejeitado na validação de segurança e não será aplicado. O arquivo de " +
        "diagnóstico traz o motivo.",
      "err.name_mismatch": "O nome digitado não corresponde ao do dispositivo.",
      "err.device_blocked": "As travas de segurança recusam este dispositivo.",
      "err.open_failed": "Não foi possível abrir o navegador.",
      "err.open_log": "Não foi possível abrir o arquivo de diagnóstico.",
      "err.scan.not_writable":
        "O dispositivo foi aberto somente para leitura. A inspeção precisa gravar.",
      "err.scan.all_writes_failed":
        "O dispositivo recusou todas as gravações. Isso não é defeito de mídia: " +
        "provavelmente o volume continua montado, a trava de gravação do cartão está " +
        "acionada, ou o programa não está rodando como Administrador.",
    },

    /* ─────────────────────────────────────────────────────────── English */
    en: {
      "lang.name": "English",

      "app.tagline": "Sector inspection and defective-area fencing",
      "splash.loading": "Loading…",

      "ui.refresh": "Refresh devices",
      "ui.language": "Language",
      "ui.device": "Device",
      "ui.selectCard": "Select the card",
      "ui.searching": "Searching…",
      "ui.choose": "Select…",
      "ui.blockedSuffix": " — blocked",
      "ui.listFailed": "Listing failed",
      "ui.noDevice": "No device",
      "ui.noCardDetected":
        "No card detected. Insert the microSD and click <strong>Refresh devices</strong>.",
      "ui.inspection": "Inspection",
      "ui.inspectionHint":
        "Writes a pattern to every sector and checks them one by one, back to front. Paints " +
        "<strong>green</strong> whatever comes back intact and <strong>red</strong> whatever " +
        "fails. It is the only way to approve area to hold data — and it erases everything " +
        "on the card.",
      "ui.inspect": "Inspect card",
      "ui.cancel": "Cancel",
      "ui.result": "Result",
      "ui.resultEmpty": "The result appears when the inspection finishes.",
      "ui.evidence": "Measurement evidence",
      "ui.evidenceCount": "{n} items",
      "ui.isolation": "Fencing",
      "ui.isolationEmpty":
        "After the inspection, the approved area can be separated from the defective one.",
      "ui.filesystem": "File system for the visible area",
      "ui.applyPlan": "Apply layout",
      "ui.plansLabel": "Available layouts",
      "ui.failure": "Interface failure: {msg}",
      "ui.thecard": "the card",
      "ui.sectorStates": "Sectors by state",

      "spec.capacity": "Capacity",
      "spec.bus": "Bus",
      "spec.sectors": "Sectors",
      "spec.volumes": "Volumes",
      "spec.of": "of {total}",
      "spec.none": "none",
      "spec.removable": "removable",
      "spec.fixed": "fixed",

      "fs.exfat": "exFAT — files over 4 GB",
      "fs.fat32": "FAT32 — maximum compatibility",

      "verdict.allowed": "Cleared for inspection",
      "verdict.needs_confirmation": "Requires confirmation",
      "verdict.blocked": "Blocked by the safety guards",
      "verdict.noNotes": "No caveats.",
      "verdict.usable": "usable",
      "verdict.split": "{good}% intact · {bad}% defective",
      "verdict.splitUnverified": " · {rest}% unverified",
      "verdict.splitTotal": " — of {total}",

      "prior.title": "Salvage has already fenced this card.",
      "prior.body":
        "The inspection will cover only the <strong>{remaining}</strong> that were left. " +
        "The {fenced} already fenced stay out of it: an earlier pass condemned them, and " +
        "there is nothing to learn there again.",
      "prior.scattered":
        "The area in use is split across {count} partitions, and the inspection covers the " +
        "interval from the first to the last — quarantine in between included. Leaving one " +
        "out is what will not do: it would sit mounted having never been checked.",

      "stage.selectFirst": "Select a card and start the inspection",
      "stage.blocked": "This device is blocked by the safety guards.",
      "stage.ready": "{name} · {size} — ready for inspection.",

      "phase.waiting": "Waiting",
      "phase.idle": "Waiting",
      "phase.writing": "Writing the pattern — back to front",
      "phase.verifying": "Reading back and comparing — front to back",
      "phase.refining": "Isolating the responsible sectors by bisection",
      "phase.done": "Inspection complete",
      "progress.of": "{done} of {total}",
      "progress.defects": " · {n} defective sectors",
      "progress.rate": " · {speed}/s · {time} left",
      "progress.inspected": "{size} inspected",
      "dur.seconds": "{n} s",
      "dur.minutes": "{n} min",
      "dur.hours": "{h} h {m} min",

      "state.0": "Unverified",
      "state.1": "Intact",
      "state.2": "Read error",
      "state.3": "Write error",
      "state.4": "Returned altered content",
      "state.5": "Returned another address's data",
      "state.6": "Fenced — holds no data",

      "scenario.pristine": "No defect found",
      "scenario.counterfeit_capacity": "Counterfeit capacity",
      "scenario.exhausted_spare": "Spare blocks exhausted",
      "scenario.actively_degrading": "Actively degrading",
      "scenario.indeterminate": "Defects found, stability unknown",
      "scenario.not_proven": "Incomplete inspection",

      "mechanism.pristine": "Every sector was written and read back identical.",
      "mechanism.counterfeit_capacity":
        "The card claims more memory than it has and serves high addresses from low cells. " +
        "The boundary comes from the firmware and does not move with use.",
      "mechanism.exhausted_spare":
        "The controller has spent the spare blocks it used to substitute for bad cells. " +
        "With no spares left, the defects froze at fixed addresses.",
      "mechanism.actively_degrading":
        "Sectors that passed the first pass failed the second. The cells are dying now, " +
        "during the inspection itself.",
      "mechanism.indeterminate":
        "The defects are real, but a single pass cannot tell a stable defect from " +
        "deterioration in progress.",
      "mechanism.not_proven":
        "The inspection did not cover the whole card. About the sectors it never reached, " +
        "nothing is known.",

      "assurance.high": "High confidence",
      "assurance.moderate": "Moderate confidence",
      "assurance.low": "Low confidence",
      "assurance.none": "No basis for confidence",

      "statement.high":
        "The boundary between the good area and the bad one is imposed by the firmware and " +
        "does not move with use. What was approved will stay approved.",
      "statement.moderate":
        "The defects appear stable, but the card is at the end of its life. Use it for data " +
        "you already have copied elsewhere.",
      "statement.low":
        "The fencing covers the defects already known. New ones may appear at addresses " +
        "approved today. Nothing irreplaceable belongs here.",
      "statement.none":
        "The measurement supports no conclusion at all: either the card got worse during " +
        "the inspection, or the inspection did not cover the whole card.",

      "detail.largestRun": "Largest contiguous approved run: <strong>{size}</strong>",
      "detail.announced_capacity": "Advertised capacity: {size}",
      "detail.real_capacity": "Estimated real capacity: {size}",
      "detail.alias_evidence": "Addresses that returned another address's data: {n}",
      "detail.defect_regions": "Distinct defective regions: {n}",
      "detail.second_pass_identical": "Second pass: the same defects, in the same places",
      "detail.newly_failed_sectors": "Sectors that passed pass 1 and failed pass 2: {size}",
      "detail.new_regions": "Regions that appeared between the two passes: {n}",
      "detail.unverified_area": "Unverified area: {size}",
      "detail.one_pass_only": "Passes run: 1 — not enough to judge stability",

      "strategy.largest_contiguous": "Single contiguous run",
      "strategy.maximum_space": "Several separate runs",
      "strategy.conservative": "Single run, widened guard band",
      "strategy.spliced_fat32": "Spliced volume (FAT32)",

      "cost.largest_contiguous": "1 drive · with guard band",
      "cost.maximum_space": "several drives · with guard band",
      "cost.conservative": "1 drive · 4× guard band",
      "cost.spliced_fat32": "1 drive · no guard band · file ≤ 4 GB",

      "note.largest_contiguous":
        "One drive over the largest contiguous approved run. The rest of the card goes into " +
        "partitions Windows will not mount. Keeps the guard band around every defect.",
      "note.maximum_space":
        "One drive for every approved run large enough to hold one. Reclaims more space " +
        "than the single run, at the cost of several drive letters. Keeps the guard band.",
      "note.conservative":
        "Same as the single contiguous run, with a guard band four times wider around the " +
        "defects. Less space, more distance between your data and the dead cell.",
      "note.spliced_fat32":
        "A single drive whose free space is the sum of every approved run. Every cluster " +
        "touching a defective sector is marked in the file system's own table and never " +
        "handed to a file. No guard band: the neighbour of a dead cell stays available. " +
        "Individual files limited to 4 GB.",

      "plan.visible": "visible",
      "plan.hidden": "hidden",
      "plan.part": "{label} ({kind}, type {type})",
      "plan.sacrificed":
        "{size} of approved area is kept out by the guard band and block alignment.",
      "plan.fencedToast": "{size} of approved area ended up fenced by the guard band.",
      "legend.written": "Written, awaiting verification",
      "plan.cannot": "This card cannot be partitioned.",
      "refusal.lead":
        "The approved area is real, but it is in pieces. A partition occupies a " +
        "contiguous run of sectors, and every mechanism has a minimum size that " +
        "none of the pieces reaches.",
      "refusal.approved": "Approved in total",
      "refusal.inPieces": "across {n} separate runs",
      "refusal.largest": "Largest contiguous run",
      "refusal.fencedNeeds": "A fenced partition needs",
      "refusal.splicedNeeds": "A spliced FAT32 volume needs",
      "refusal.why":
        "The fenced partition needs a guard band at each end plus the minimum " +
        "partition size. The spliced volume needs that much contiguous healthy " +
        "ground at its start for the allocation table and the root directory " +
        "alone — and there is nowhere to put them.",
      "plan.cannotWhy":
        "A partition can only be created over contiguous verified area. No run large enough " +
        "was left on this card.",
      "plan.none.pristine": "No defect was found: there is no area to fence on this card.",
      "plan.none.not_proven":
        "The inspection did not cover the whole card. Run it to the end so that there is " +
        "approved area to plan over.",
      "plan.none.degrading":
        "The card lost sectors during the inspection itself. No partition layout protects " +
        "against defects that do not exist yet.",

      "modal.confirmTitle": "Confirm destructive operation",
      "modal.typeName": "To confirm, type the device name:",
      "modal.confirm": "Confirm",
      "modal.cancel": "Cancel",
      "modal.close": "Close",

      "scan.confirmTitle": "Confirm destructive inspection",
      "scan.scopeAll": "to every sector of {name} ({size})",
      "scan.scopePrior": "to the {size} the last inspection left in use on {name}",
      "scan.confirmBody":
        "<p class=\"destructive\">The inspection writes {scope}. All content will be " +
        "lost.</p><p>Every sector is written and then read back. Only what comes back " +
        "identical is approved — that is how counterfeit capacity and silent corruption " +
        "surface, and neither shows itself any other way.</p>",
      "scan.warningTitle": "Note:",
      "scan.cancelled": "Inspection cancelled.",
      "scan.done": "Inspection complete.",
      "scan.watchdog":
        "The inspection started but reported no progress within {n}s. Check the diagnostic " +
        "file named in the status bar.",

      "apply.confirmTitle": "Confirm repartitioning",
      "apply.erases":
        "This operation erases everything on {name} and writes a new partition table.",
      "apply.layoutIs": "Layout <strong>{name}</strong>:",
      "apply.dataParts":
        "<strong>{n}</strong> visible partition(s), totalling <strong>{size}</strong>, over " +
        "area approved sector by sector.",
      "apply.hiddenParts":
        "<strong>{n}</strong> hidden partition(s) of type 0xDA, which Windows neither mounts " +
        "nor displays.",
      "apply.caveat":
        "Fencing lowers the risk, but it does not turn a failing card into a reliable one. " +
        "Keep a copy of anything irreplaceable.",
      "apply.applying": "Applying the layout… do not remove the card.",
      "apply.doneTitle": "Layout applied",
      "apply.where": "The reliable area is available at <strong>{letter}:</strong>",
      "apply.done": "Repartitioning complete.",

      "close.title": "Inspection in progress",
      "close.body":
        "<p class=\"destructive\">An inspection is running on {name}.</p>" +
        "<p>Closing now interrupts the pass. What has already been written to the card does " +
        "not come back, and nothing ends up approved: the inspection only approves area it " +
        "read back in full.</p>" +
        "<p>The program waits for the inspection to release the card before it exits, so " +
        "that Windows can see it again.</p>",
      "close.stop": "Stop and close",
      "close.keep": "Continue the inspection",
      "close.leaving": "Exiting: waiting for the inspection to release the card…",

      "step.volumes_dismounted": "Existing volumes locked and dismounted.",
      "step.volume_warning": "Warning: {detail}",
      "step.table_changed":
        "Entry {slot}: {action} — type {type}, sectors {from} to {to} ({n} sectors)",
      "step.table_changed.added": "created",
      "step.table_changed.removed": "removed",
      "step.partition_head_wiped":
        "The head of partition '{label}' was zeroed, so that no old file system is " +
        "recognised over the new layout.",
      "step.table_written": "New partition table written to sector 0.",
      "step.system_notified": "Windows told to re-read the disk layout.",
      "step.volume_mounted": "Data partition mounted at {letter}:",
      "step.formatted": "{letter}: formatted as {filesystem}.",
      "step.cluster_map_written":
        "FAT32 volume written with {n} clusters marked defective in the allocation table. " +
        "No driver hands those clusters to a file.",
      "step.mount_timed_out":
        "The partition was created, but Windows did not mount it in time. Remove and " +
        "reinsert the card to finish formatting.",

      "block.hosts_operating_system":
        "This disk holds Windows (volumes {volumes}). Writing to it leaves the machine " +
        "unable to boot.",
      "block.hosts_operating_system.system": "system",
      "block.internal_bus":
        "Connected over {bus}, a bus used only by internal disks. No card reader presents " +
        "itself this way.",
      "block.exceeds_addressable_capacity":
        "Capacity of {capacity}, above the {limit} limit this program addresses.",
      "block.zero_capacity":
        "The device reports zero capacity: either no media is inserted, or it stopped " +
        "responding.",

      "warn.not_declared_removable":
        "Windows does not mark this media as removable. USB microSD adapters commonly " +
        "present themselves this way, but confirm the target.",
      "warn.has_mounted_volumes": "Volumes {volumes} are mounted and will be lost.",
      "warn.unusually_large":
        "A capacity of {capacity} is high for a card. Confirm the target is the one you " +
        "mean to erase.",
      "warn.unknown_bus":
        "The bus this device is connected over could not be identified, so the checks that " +
        "depend on it did not apply.",

      "footer.diagnostics": "Diagnostics:",
      "footer.by": "by ",
      "footer.openLog": "Open the diagnostic file",

      "err.state": "The program's internal state was corrupted. Close it and open it again.",
      "err.enumerate":
        "The devices could not be listed. The program needs Administrator privilege to open " +
        "disks.",
      "err.device_gone": "Device not found; refresh the list.",
      "err.scan_running": "An inspection is already running.",
      "err.no_device": "Select a device first.",
      "err.no_map": "Inspect the card before planning.",
      "err.no_plan": "That layout no longer exists; compute the layouts again.",
      "err.plan_rejected":
        "The layout was rejected by safety validation and will not be applied. The " +
        "diagnostic file carries the reason.",
      "err.name_mismatch": "The name typed does not match the device's.",
      "err.device_blocked": "The safety guards refuse this device.",
      "err.open_failed": "The browser could not be opened.",
      "err.open_log": "The diagnostic file could not be opened.",
      "err.scan.not_writable":
        "The device was opened read-only. The inspection needs to write.",
      "err.scan.all_writes_failed":
        "The device refused every write. This is not a media defect: the volume is probably " +
        "still mounted, the card's write-protect switch is engaged, or the program is not " +
        "running as Administrator.",
    },

    /* ─────────────────────────────────────────────────────────── Spanish */
    es: {
      "lang.name": "Español",

      "app.tagline": "Inspección de sectores y aislamiento del área defectuosa",
      "splash.loading": "Cargando…",

      "ui.refresh": "Actualizar dispositivos",
      "ui.language": "Idioma",
      "ui.device": "Dispositivo",
      "ui.selectCard": "Seleccione la tarjeta",
      "ui.searching": "Buscando…",
      "ui.choose": "Seleccione…",
      "ui.blockedSuffix": " — bloqueado",
      "ui.listFailed": "Error al listar",
      "ui.noDevice": "Ningún dispositivo",
      "ui.noCardDetected":
        "No se detectó ninguna tarjeta. Inserte la microSD y pulse <strong>Actualizar " +
        "dispositivos</strong>.",
      "ui.inspection": "Inspección",
      "ui.inspectionHint":
        "Escribe un patrón en todos los sectores y los comprueba uno por uno, del final al " +
        "principio. Pinta de <strong>verde</strong> lo que vuelve intacto y de " +
        "<strong>rojo</strong> lo que falla. Es la única forma de aprobar área para guardar " +
        "datos — y borra todo el contenido de la tarjeta.",
      "ui.inspect": "Inspeccionar tarjeta",
      "ui.cancel": "Cancelar",
      "ui.result": "Resultado",
      "ui.resultEmpty": "El resultado aparece cuando termina la inspección.",
      "ui.evidence": "Evidencia de la medición",
      "ui.evidenceCount": "{n} elementos",
      "ui.isolation": "Aislamiento",
      "ui.isolationEmpty":
        "Después de la inspección se puede separar el área aprobada de la defectuosa.",
      "ui.filesystem": "Sistema de archivos del área visible",
      "ui.applyPlan": "Aplicar diseño",
      "ui.plansLabel": "Diseños disponibles",
      "ui.failure": "Fallo en la interfaz: {msg}",
      "ui.thecard": "la tarjeta",
      "ui.sectorStates": "Sectores por estado",

      "spec.capacity": "Capacidad",
      "spec.bus": "Bus",
      "spec.sectors": "Sectores",
      "spec.volumes": "Volúmenes",
      "spec.of": "de {total}",
      "spec.none": "ninguno",
      "spec.removable": "extraíble",
      "spec.fixed": "fijo",

      "fs.exfat": "exFAT — archivos de más de 4 GB",
      "fs.fat32": "FAT32 — compatibilidad máxima",

      "verdict.allowed": "Autorizado para inspección",
      "verdict.needs_confirmation": "Requiere confirmación",
      "verdict.blocked": "Bloqueado por los seguros",
      "verdict.noNotes": "Sin reservas.",
      "verdict.usable": "aprovechables",
      "verdict.split": "{good}% íntegro · {bad}% defectuoso",
      "verdict.splitUnverified": " · {rest}% sin comprobar",
      "verdict.splitTotal": " — de {total}",

      "prior.title": "Salvage ya aisló esta tarjeta.",
      "prior.body":
        "La inspección tratará solo los <strong>{remaining}</strong> que quedaron. Los " +
        "{fenced} ya aislados quedan fuera: fueron condenados en una pasada anterior y no " +
        "hay nada que volver a aprender en ellos.",
      "prior.scattered":
        "El área en uso está repartida en {count} particiones, y la inspección cubre el " +
        "intervalo que va de la primera a la última — incluida la cuarentena entre ellas. " +
        "Dejar una fuera es lo que no sirve: quedaría montada sin haber sido comprobada.",

      "stage.selectFirst": "Seleccione una tarjeta e inicie la inspección",
      "stage.blocked": "Este dispositivo está bloqueado por los seguros.",
      "stage.ready": "{name} · {size} — listo para inspección.",

      "phase.waiting": "Esperando",
      "phase.idle": "Esperando",
      "phase.writing": "Escribiendo el patrón — del final al principio",
      "phase.verifying": "Releyendo y comparando — del principio al final",
      "phase.refining": "Aislando los sectores responsables por bisección",
      "phase.done": "Inspección terminada",
      "progress.of": "{done} de {total}",
      "progress.defects": " · {n} sectores defectuosos",
      "progress.rate": " · {speed}/s · quedan {time}",
      "progress.inspected": "{size} inspeccionados",
      "dur.seconds": "{n} s",
      "dur.minutes": "{n} min",
      "dur.hours": "{h} h {m} min",

      "state.0": "Sin comprobar",
      "state.1": "Íntegro",
      "state.2": "Error de lectura",
      "state.3": "Error de escritura",
      "state.4": "Devolvió contenido alterado",
      "state.5": "Devolvió datos de otra dirección",
      "state.6": "Aislado — no recibe datos",

      "scenario.pristine": "Ningún defecto encontrado",
      "scenario.counterfeit_capacity": "Capacidad falsificada",
      "scenario.exhausted_spare": "Bloques de reserva agotados",
      "scenario.actively_degrading": "Degradación activa",
      "scenario.indeterminate": "Defectos encontrados, estabilidad desconocida",
      "scenario.not_proven": "Inspección incompleta",

      "mechanism.pristine": "Todos los sectores se escribieron y se releyeron idénticos.",
      "mechanism.counterfeit_capacity":
        "La tarjeta anuncia más memoria de la que tiene y sirve direcciones altas con " +
        "celdas bajas. La frontera viene del firmware y no se mueve con el uso.",
      "mechanism.exhausted_spare":
        "La controladora gastó los bloques de reserva que usaba para sustituir celdas " +
        "malas. Sin reserva, los defectos quedaron congelados en direcciones fijas.",
      "mechanism.actively_degrading":
        "Sectores aprobados en la primera pasada fallaron en la segunda. Las celdas están " +
        "muriendo ahora, durante la propia inspección.",
      "mechanism.indeterminate":
        "Los defectos son reales, pero una sola pasada no distingue un defecto estable de " +
        "un deterioro en curso.",
      "mechanism.not_proven":
        "La inspección no cubrió toda la tarjeta. Sobre los sectores que no alcanzó, no se " +
        "sabe nada.",

      "assurance.high": "Confianza alta",
      "assurance.moderate": "Confianza moderada",
      "assurance.low": "Confianza baja",
      "assurance.none": "Sin base para confiar",

      "statement.high":
        "La frontera entre el área buena y la mala la impone el firmware y no se mueve con " +
        "el uso. Lo aprobado seguirá aprobado.",
      "statement.moderate":
        "Los defectos parecen estables, pero la tarjeta está al final de su vida útil. Úsela " +
        "para datos que ya tenga copiados en otro sitio.",
      "statement.low":
        "El aislamiento cubre los defectos ya conocidos. Pueden aparecer otros en " +
        "direcciones hoy aprobadas. Nada irremplazable debería quedarse aquí.",
      "statement.none":
        "La medición no sostiene ninguna conclusión: o la tarjeta empeoró durante la propia " +
        "inspección, o esta no cubrió toda la tarjeta.",

      "detail.largestRun": "Mayor tramo continuo aprobado: <strong>{size}</strong>",
      "detail.announced_capacity": "Capacidad anunciada: {size}",
      "detail.real_capacity": "Capacidad real estimada: {size}",
      "detail.alias_evidence": "Direcciones que devolvieron datos de otra dirección: {n}",
      "detail.defect_regions": "Regiones defectuosas distintas: {n}",
      "detail.second_pass_identical": "Segunda pasada: los mismos defectos, en los mismos sitios",
      "detail.newly_failed_sectors":
        "Sectores aprobados en la 1.ª pasada que fallaron en la 2.ª: {size}",
      "detail.new_regions": "Regiones que surgieron entre las dos pasadas: {n}",
      "detail.unverified_area": "Área sin comprobar: {size}",
      "detail.one_pass_only": "Pasadas ejecutadas: 1 — insuficiente para juzgar la estabilidad",

      "strategy.largest_contiguous": "Franja contigua única",
      "strategy.maximum_space": "Varias franjas separadas",
      "strategy.conservative": "Franja única, margen ampliado",
      "strategy.spliced_fat32": "Volumen empalmado (FAT32)",

      "cost.largest_contiguous": "1 unidad · con margen",
      "cost.maximum_space": "varias unidades · con margen",
      "cost.conservative": "1 unidad · margen 4×",
      "cost.spliced_fat32": "1 unidad · sin margen · archivo ≤ 4 GB",

      "note.largest_contiguous":
        "Una unidad sobre el mayor tramo continuo aprobado. El resto de la tarjeta va a " +
        "particiones que Windows no monta. Mantiene el margen de seguridad alrededor de " +
        "cada defecto.",
      "note.maximum_space":
        "Una unidad por cada tramo aprobado lo bastante grande. Aprovecha más espacio que " +
        "la franja única, al coste de varias letras de unidad. Mantiene el margen de " +
        "seguridad.",
      "note.conservative":
        "Igual que la franja contigua única, con un margen cuatro veces mayor alrededor de " +
        "los defectos. Menos espacio, más distancia entre sus datos y la celda muerta.",
      "note.spliced_fat32":
        "Una sola unidad cuyo espacio libre es la suma de todos los tramos aprobados. Cada " +
        "clúster que toca un sector defectuoso se marca en la tabla del sistema de archivos " +
        "y nunca se entrega a un archivo. Sin margen de seguridad: el vecino de una celda " +
        "muerta sigue disponible. Archivo individual limitado a 4 GB.",

      "plan.visible": "visible",
      "plan.hidden": "oculta",
      "plan.part": "{label} ({kind}, tipo {type})",
      "plan.sacrificed":
        "{size} de área aprobada quedan fuera por el margen de seguridad y la alineación de " +
        "bloque.",
      "plan.fencedToast":
        "{size} de área aprobada quedaron cercados por el margen de seguridad.",
      "legend.written": "Escrito, pendiente de verificación",
      "plan.cannot": "Esta tarjeta no se puede particionar.",
      "refusal.lead":
        "El área aprobada existe, pero está fragmentada. Una partición ocupa un " +
        "tramo contiguo de sectores, y cada mecanismo tiene un tamaño mínimo que " +
        "ninguno de los fragmentos alcanza.",
      "refusal.approved": "Aprobado en total",
      "refusal.inPieces": "en {n} tramos separados",
      "refusal.largest": "Mayor tramo contiguo",
      "refusal.fencedNeeds": "Una partición cercada necesita",
      "refusal.splicedNeeds": "Un volumen FAT32 empalmado necesita",
      "refusal.why":
        "La partición cercada necesita el margen de seguridad a ambos lados más " +
        "el tamaño mínimo. El volumen empalmado necesita ese espacio contiguo e " +
        "íntegro al principio, solo para la tabla de asignación y el directorio " +
        "raíz — y no hay dónde ponerlos.",
      "plan.cannotWhy":
        "Una partición solo puede crearse sobre área contigua verificada. En esta tarjeta " +
        "no quedó ninguna lo bastante grande.",
      "plan.none.pristine":
        "No se encontró ningún defecto: no hay área que aislar en esta tarjeta.",
      "plan.none.not_proven":
        "La inspección no cubrió toda la tarjeta. Ejecútela hasta el final para que haya " +
        "área aprobada sobre la que planificar.",
      "plan.none.degrading":
        "La tarjeta perdió sectores durante la propia inspección. Ningún diseño de " +
        "particiones protege contra defectos que todavía no existen.",

      "modal.confirmTitle": "Confirmar operación destructiva",
      "modal.typeName": "Para confirmar, escriba el nombre del dispositivo:",
      "modal.confirm": "Confirmar",
      "modal.cancel": "Cancelar",
      "modal.close": "Cerrar",

      "scan.confirmTitle": "Confirmar inspección destructiva",
      "scan.scopeAll": "en todos los sectores de {name} ({size})",
      "scan.scopePrior": "en los {size} que la última inspección dejó en uso en {name}",
      "scan.confirmBody":
        "<p class=\"destructive\">La inspección escribe {scope}. Se perderá todo el " +
        "contenido.</p><p>Cada sector se escribe y luego se relee. Solo lo que vuelve " +
        "idéntico se aprueba — así es como aparecen la capacidad falsificada y la " +
        "corrupción silenciosa, y ninguna de las dos se revela de otro modo.</p>",
      "scan.warningTitle": "Atención:",
      "scan.cancelled": "Inspección cancelada.",
      "scan.done": "Inspección terminada.",
      "scan.watchdog":
        "La inspección se inició pero no informó de ningún avance en {n}s. Consulte el " +
        "archivo de diagnóstico indicado en la barra de estado.",

      "apply.confirmTitle": "Confirmar reparticionado",
      "apply.erases":
        "Esta operación borra todo lo que hay en {name} y escribe una nueva tabla de " +
        "particiones.",
      "apply.layoutIs": "Diseño <strong>{name}</strong>:",
      "apply.dataParts":
        "<strong>{n}</strong> partición(es) visible(s), sumando <strong>{size}</strong>, " +
        "sobre área aprobada sector por sector.",
      "apply.hiddenParts":
        "<strong>{n}</strong> partición(es) oculta(s) de tipo 0xDA, que Windows ni monta ni " +
        "muestra.",
      "apply.caveat":
        "El aislamiento reduce el riesgo, pero no convierte una tarjeta defectuosa en " +
        "fiable. Guarde copia de cualquier dato irremplazable.",
      "apply.applying": "Aplicando el diseño… no retire la tarjeta.",
      "apply.doneTitle": "Diseño aplicado",
      "apply.where": "El área fiable está disponible en <strong>{letter}:</strong>",
      "apply.done": "Reparticionado terminado.",

      "close.title": "Inspección en curso",
      "close.body":
        "<p class=\"destructive\">Hay una inspección en curso en {name}.</p>" +
        "<p>Cerrar ahora interrumpe la pasada. Lo que ya se escribió en la tarjeta no vuelve " +
        "atrás y nada queda aprobado: la inspección solo aprueba área que ella misma releyó " +
        "por completo.</p>" +
        "<p>El programa espera a que la inspección suelte la tarjeta antes de cerrarse, " +
        "para que Windows vuelva a verla.</p>",
      "close.stop": "Detener y cerrar",
      "close.keep": "Continuar la inspección",
      "close.leaving": "Cerrando: esperando a que la inspección suelte la tarjeta…",

      "step.volumes_dismounted": "Volúmenes existentes bloqueados y desmontados.",
      "step.volume_warning": "Aviso: {detail}",
      "step.table_changed":
        "Entrada {slot}: {action} — tipo {type}, sectores {from} a {to} ({n} sectores)",
      "step.table_changed.added": "creada",
      "step.table_changed.removed": "eliminada",
      "step.partition_head_wiped":
        "Se puso a cero el inicio de la partición '{label}', para que ningún sistema de " +
        "archivos antiguo se reconozca sobre el nuevo diseño.",
      "step.table_written": "Nueva tabla de particiones escrita en el sector 0.",
      "step.system_notified": "Se avisó a Windows para releer el diseño del disco.",
      "step.volume_mounted": "Partición de datos montada en {letter}:",
      "step.formatted": "{letter}: formateada como {filesystem}.",
      "step.cluster_map_written":
        "Volumen FAT32 escrito con {n} clústeres marcados como defectuosos en la tabla de " +
        "asignación. Ningún controlador entrega esos clústeres a un archivo.",
      "step.mount_timed_out":
        "La partición se creó, pero Windows no la montó a tiempo. Retire y vuelva a insertar " +
        "la tarjeta para terminar el formateo.",

      "block.hosts_operating_system":
        "Este disco contiene Windows (volúmenes {volumes}). Escribir en él deja la máquina " +
        "sin arrancar.",
      "block.hosts_operating_system.system": "de sistema",
      "block.internal_bus":
        "Conectado por {bus}, un bus que solo usan los discos internos. Ningún lector de " +
        "tarjetas se presenta así.",
      "block.exceeds_addressable_capacity":
        "Capacidad de {capacity}, por encima del límite de {limit} que el programa " +
        "direcciona.",
      "block.zero_capacity":
        "El dispositivo responde con capacidad cero: no hay medio insertado, o dejó de " +
        "responder.",

      "warn.not_declared_removable":
        "Windows no marca este medio como extraíble. Los adaptadores USB de microSD suelen " +
        "presentarse así, pero confirme el destino.",
      "warn.has_mounted_volumes": "Los volúmenes {volumes} están montados y se perderán.",
      "warn.unusually_large":
        "Una capacidad de {capacity} es alta para una tarjeta. Confirme que el destino es el " +
        "que pretende borrar.",
      "warn.unknown_bus":
        "No se pudo identificar por qué bus está conectado el dispositivo, así que las " +
        "comprobaciones que dependen de eso no se aplicaron.",

      "footer.diagnostics": "Diagnóstico:",
      "footer.by": "por ",
      "footer.openLog": "Abrir el archivo de diagnóstico",

      "err.state": "El estado interno del programa se corrompió. Ciérrelo y ábralo de nuevo.",
      "err.enumerate":
        "No se pudieron listar los dispositivos. El programa necesita privilegios de " +
        "Administrador para abrir discos.",
      "err.device_gone": "Dispositivo no encontrado; actualice la lista.",
      "err.scan_running": "Ya hay una inspección en curso.",
      "err.no_device": "Seleccione un dispositivo primero.",
      "err.no_map": "Inspeccione la tarjeta antes de planificar.",
      "err.no_plan": "Ese diseño ya no existe; calcule los diseños de nuevo.",
      "err.plan_rejected":
        "El diseño fue rechazado en la validación de seguridad y no se aplicará. El archivo " +
        "de diagnóstico trae el motivo.",
      "err.name_mismatch": "El nombre escrito no coincide con el del dispositivo.",
      "err.device_blocked": "Los seguros rechazan este dispositivo.",
      "err.open_failed": "No se pudo abrir el navegador.",
      "err.open_log": "No se pudo abrir el archivo de diagnóstico.",
      "err.scan.not_writable":
        "El dispositivo se abrió solo para lectura. La inspección necesita escribir.",
      "err.scan.all_writes_failed":
        "El dispositivo rechazó todas las escrituras. Esto no es un defecto del medio: " +
        "probablemente el volumen sigue montado, el seguro de escritura de la tarjeta está " +
        "activado, o el programa no se está ejecutando como Administrador.",
    },

    /* ────────────────────────────────────────────────────── Chinese (Simplified) */
    zh: {
      "lang.name": "中文",

      "app.tagline": "扇区检测与缺陷区域隔离",
      "splash.loading": "正在加载…",

      "ui.refresh": "刷新设备",
      "ui.language": "语言",
      "ui.device": "设备",
      "ui.selectCard": "选择存储卡",
      "ui.searching": "正在查找…",
      "ui.choose": "请选择…",
      "ui.blockedSuffix": " — 已阻止",
      "ui.listFailed": "列举失败",
      "ui.noDevice": "无设备",
      "ui.noCardDetected": "未检测到存储卡。请插入 microSD 卡并点击<strong>刷新设备</strong>。",
      "ui.inspection": "检测",
      "ui.inspectionHint":
        "向每一个扇区写入图案，再由后向前逐个校验。原样返回的涂成<strong>绿色</strong>，" +
        "失败的涂成<strong>红色</strong>。这是唯一能认定某片区域可以存放数据的办法 —— " +
        "并且会清除卡上的全部内容。",
      "ui.inspect": "检测存储卡",
      "ui.cancel": "取消",
      "ui.result": "结果",
      "ui.resultEmpty": "检测结束后在此显示结果。",
      "ui.evidence": "测量依据",
      "ui.evidenceCount": "{n} 项",
      "ui.isolation": "隔离",
      "ui.isolationEmpty": "检测完成后即可把合格区域与缺陷区域分开。",
      "ui.filesystem": "可见区域的文件系统",
      "ui.applyPlan": "应用布局",
      "ui.plansLabel": "可选布局",
      "ui.failure": "界面故障：{msg}",
      "ui.thecard": "该存储卡",
      "ui.sectorStates": "按状态统计的扇区",

      "spec.capacity": "容量",
      "spec.bus": "总线",
      "spec.sectors": "扇区",
      "spec.volumes": "卷",
      "spec.of": "共 {total}",
      "spec.none": "无",
      "spec.removable": "可移动",
      "spec.fixed": "固定",

      "fs.exfat": "exFAT — 支持大于 4 GB 的文件",
      "fs.fat32": "FAT32 — 兼容性最好",

      "verdict.allowed": "允许检测",
      "verdict.needs_confirmation": "需要确认",
      "verdict.blocked": "已被安全保护阻止",
      "verdict.noNotes": "无需注意事项。",
      "verdict.usable": "可用",
      "verdict.split": "{good}% 完好 · {bad}% 有缺陷",
      "verdict.splitUnverified": " · {rest}% 未校验",
      "verdict.splitTotal": " — 共 {total}",

      "prior.title": "Salvage 已经隔离过这张卡。",
      "prior.body":
        "检测只处理剩余的 <strong>{remaining}</strong>。已隔离的 {fenced} 不在其中：" +
        "它们在上一次检测中已被判定为缺陷，没有什么需要重新确认。",
      "prior.scattered":
        "使用中的区域分布在 {count} 个分区上，检测覆盖从第一个到最后一个之间的整段区间 —— " +
        "包括其间的隔离区。漏掉其中任何一个才是不行的：那会让一个已挂载的分区从未被校验。",

      "stage.selectFirst": "请选择一张卡并开始检测",
      "stage.blocked": "该设备已被安全保护阻止。",
      "stage.ready": "{name} · {size} — 可以开始检测。",

      "phase.waiting": "等待中",
      "phase.idle": "等待中",
      "phase.writing": "正在写入图案 —— 由后向前",
      "phase.verifying": "正在回读比对 —— 由前向后",
      "phase.refining": "正在用二分法定位出错扇区",
      "phase.done": "检测完成",
      "progress.of": "{done} / {total}",
      "progress.defects": " · {n} 个缺陷扇区",
      "progress.rate": " · {speed}/秒 · 剩余 {time}",
      "progress.inspected": "已检测 {size}",
      "dur.seconds": "{n} 秒",
      "dur.minutes": "{n} 分钟",
      "dur.hours": "{h} 小时 {m} 分",

      "state.0": "未校验",
      "state.1": "完好",
      "state.2": "读取错误",
      "state.3": "写入错误",
      "state.4": "返回内容被改动",
      "state.5": "返回了其他地址的数据",
      "state.6": "已隔离 —— 不存放数据",

      "scenario.pristine": "未发现缺陷",
      "scenario.counterfeit_capacity": "容量造假",
      "scenario.exhausted_spare": "备用块已耗尽",
      "scenario.actively_degrading": "正在持续劣化",
      "scenario.indeterminate": "发现缺陷，稳定性未知",
      "scenario.not_proven": "检测不完整",

      "mechanism.pristine": "所有扇区写入后回读均完全一致。",
      "mechanism.counterfeit_capacity":
        "这张卡声称的容量大于实际，用低地址的存储单元冒充高地址。这条界限由固件决定，" +
        "不会随使用而移动。",
      "mechanism.exhausted_spare":
        "控制器已经用尽了用来替换坏单元的备用块。没有备用块之后，缺陷就固定在了这些地址上。",
      "mechanism.actively_degrading":
        "第一遍通过的扇区在第二遍失败了。这些存储单元正在损坏 —— 就在检测进行的过程中。",
      "mechanism.indeterminate": "缺陷是真实的，但仅一遍检测无法区分稳定缺陷与正在发生的劣化。",
      "mechanism.not_proven": "检测没有覆盖整张卡。对于未触及的扇区，一无所知。",

      "assurance.high": "高可信度",
      "assurance.moderate": "中等可信度",
      "assurance.low": "低可信度",
      "assurance.none": "没有可信的依据",

      "statement.high":
        "好区域与坏区域之间的界限由固件强制划定，不会随使用而移动。已认定合格的仍将合格。",
      "statement.moderate":
        "缺陷看起来是稳定的，但这张卡已接近寿命终点。只用来存放你在别处已有备份的数据。",
      "statement.low":
        "隔离覆盖的是已知的缺陷。今天合格的地址上仍可能出现新的缺陷。不要在这里放无可替代的东西。",
      "statement.none":
        "这次测量得不出任何结论：要么卡在检测过程中变得更坏，要么检测没有覆盖整张卡。",

      "detail.largestRun": "最大连续合格区段：<strong>{size}</strong>",
      "detail.announced_capacity": "标称容量：{size}",
      "detail.real_capacity": "估算的实际容量：{size}",
      "detail.alias_evidence": "返回了其他地址数据的地址数：{n}",
      "detail.defect_regions": "独立缺陷区域数：{n}",
      "detail.second_pass_identical": "第二遍：同样的缺陷，出现在同样的位置",
      "detail.newly_failed_sectors": "第一遍通过、第二遍失败的扇区：{size}",
      "detail.new_regions": "两遍之间新出现的区域数：{n}",
      "detail.unverified_area": "未校验区域：{size}",
      "detail.one_pass_only": "已执行遍数：1 —— 不足以判断稳定性",

      "strategy.largest_contiguous": "单一连续区段",
      "strategy.maximum_space": "多个独立区段",
      "strategy.conservative": "单一区段，加宽安全边界",
      "strategy.spliced_fat32": "拼接卷（FAT32）",

      "cost.largest_contiguous": "1 个驱动器 · 带安全边界",
      "cost.maximum_space": "多个驱动器 · 带安全边界",
      "cost.conservative": "1 个驱动器 · 4 倍安全边界",
      "cost.spliced_fat32": "1 个驱动器 · 无安全边界 · 单文件 ≤ 4 GB",

      "note.largest_contiguous":
        "在最大的连续合格区段上建立一个驱动器。卡上其余部分划入 Windows 不会挂载的分区。" +
        "在每处缺陷周围保留安全边界。",
      "note.maximum_space":
        "为每一段足够大的合格区域各建一个驱动器。比单一区段回收更多空间，代价是占用多个盘符。" +
        "保留安全边界。",
      "note.conservative":
        "与单一连续区段相同，但缺陷周围的安全边界扩大到四倍。空间更少，你的数据与坏单元之间的距离更远。",
      "note.spliced_fat32":
        "只建一个驱动器，其可用空间是所有合格区段之和。凡是接触到缺陷扇区的簇都会在文件系统的" +
        "分配表中标记为坏簇，永不分配给文件。没有安全边界：坏单元的相邻单元仍可使用。" +
        "单个文件不得超过 4 GB。",

      "plan.visible": "可见",
      "plan.hidden": "隐藏",
      "plan.part": "{label}（{kind}，类型 {type}）",
      "plan.sacrificed": "有 {size} 的合格区域因安全边界和块对齐而未被使用。",
      "plan.fencedToast": "有 {size} 的合格区域被安全边界圈了进去。",
      "legend.written": "已写入，等待校验",
      "plan.cannot": "这张卡无法分区。",
      "refusal.lead":
        "合格区域确实存在，但被打散了。分区占用一段连续的扇区，而每种机制都有一个最小尺寸，没有任何一段能达到。",
      "refusal.approved": "合格区域总计",
      "refusal.inPieces": "分散在 {n} 段中",
      "refusal.largest": "最大连续段",
      "refusal.fencedNeeds": "隔离分区需要",
      "refusal.splicedNeeds": "拼接的 FAT32 卷需要",
      "refusal.why":
        "隔离分区两侧各需要一条保护带，再加上最小分区尺寸。拼接卷仅为分配表和根目录就需要开头有这么多连续且完好的空间——而这张卡上无处安放。",
      "plan.cannotWhy": "分区只能建立在连续且已校验的区域上。这张卡上没有留下足够大的区段。",
      "plan.none.pristine": "未发现任何缺陷：这张卡上没有需要隔离的区域。",
      "plan.none.not_proven": "检测没有覆盖整张卡。请运行到结束，才会有可供规划的合格区域。",
      "plan.none.degrading":
        "这张卡在检测过程中就在丢失扇区。任何分区布局都无法防住尚未出现的缺陷。",

      "modal.confirmTitle": "确认破坏性操作",
      "modal.typeName": "请输入设备名称以确认：",
      "modal.confirm": "确认",
      "modal.cancel": "取消",
      "modal.close": "关闭",

      "scan.confirmTitle": "确认破坏性检测",
      "scan.scopeAll": "{name}（{size}）的每一个扇区",
      "scan.scopePrior": "{name} 上一次检测留作使用的 {size}",
      "scan.confirmBody":
        "<p class=\"destructive\">检测将写入{scope}。全部内容都会丢失。</p>" +
        "<p>每个扇区先写入、再回读。只有原样返回的才算合格 —— " +
        "容量造假和静默损坏正是这样暴露出来的，而这两者都没有别的办法能查出。</p>",
      "scan.warningTitle": "注意：",
      "scan.cancelled": "检测已取消。",
      "scan.done": "检测完成。",
      "scan.watchdog": "检测已启动，但 {n} 秒内没有报告任何进度。请查看状态栏中标明的诊断文件。",

      "apply.confirmTitle": "确认重新分区",
      "apply.erases": "此操作会清除 {name} 上的全部内容，并写入新的分区表。",
      "apply.layoutIs": "布局 <strong>{name}</strong>：",
      "apply.dataParts":
        "<strong>{n}</strong> 个可见分区，合计 <strong>{size}</strong>，" +
        "全部位于逐扇区校验合格的区域上。",
      "apply.hiddenParts": "<strong>{n}</strong> 个 0xDA 类型的隐藏分区，Windows 既不挂载也不显示。",
      "apply.caveat":
        "隔离能降低风险，但不会把一张有缺陷的卡变得可靠。任何无可替代的数据都要另存一份。",
      "apply.applying": "正在应用布局…… 请勿拔出存储卡。",
      "apply.doneTitle": "布局已应用",
      "apply.where": "可靠区域已挂载在 <strong>{letter}:</strong>",
      "apply.done": "重新分区完成。",

      "close.title": "检测正在进行",
      "close.body":
        "<p class=\"destructive\">{name} 上有一次检测正在进行。</p>" +
        "<p>现在关闭会中断这一遍。已经写入卡上的内容无法恢复，也不会有任何区域被认定合格：" +
        "检测只认定它自己完整回读过的区域。</p>" +
        "<p>程序会等检测释放存储卡之后再退出，这样 Windows 才能重新看到它。</p>",
      "close.stop": "停止并关闭",
      "close.keep": "继续检测",
      "close.leaving": "正在退出：等待检测释放存储卡……",

      "step.volumes_dismounted": "已锁定并卸载原有卷。",
      "step.volume_warning": "警告：{detail}",
      "step.table_changed": "表项 {slot}：{action} —— 类型 {type}，扇区 {from} 至 {to}（{n} 个扇区）",
      "step.table_changed.added": "已创建",
      "step.table_changed.removed": "已移除",
      "step.partition_head_wiped":
        "已将分区“{label}”的开头清零，使新布局之上不会再识别出任何旧的文件系统。",
      "step.table_written": "新的分区表已写入 0 号扇区。",
      "step.system_notified": "已通知 Windows 重新读取磁盘布局。",
      "step.volume_mounted": "数据分区已挂载在 {letter}:",
      "step.formatted": "{letter}: 已格式化为 {filesystem}。",
      "step.cluster_map_written":
        "FAT32 卷已写入，并在分配表中把 {n} 个簇标记为坏簇。任何驱动程序都不会把这些簇分配给文件。",
      "step.mount_timed_out":
        "分区已创建，但 Windows 未能及时挂载。请拔出并重新插入存储卡以完成格式化。",

      "block.hosts_operating_system": "该磁盘上装有 Windows（卷 {volumes}）。向它写入会让这台机器无法启动。",
      "block.hosts_operating_system.system": "系统",
      "block.internal_bus": "通过 {bus} 连接，这类总线只用于内置磁盘。读卡器不会这样出现。",
      "block.exceeds_addressable_capacity": "容量为 {capacity}，超过本程序可寻址的 {limit} 上限。",
      "block.zero_capacity": "设备报告容量为零：可能没有插入介质，或者介质已停止响应。",

      "warn.not_declared_removable":
        "Windows 没有把该介质标记为可移动。USB 的 microSD 读卡器常常这样出现，但请确认目标。",
      "warn.has_mounted_volumes": "卷 {volumes} 已挂载，其中的数据将会丢失。",
      "warn.unusually_large": "{capacity} 对一张存储卡来说偏大。请确认这就是你要清除的目标。",
      "warn.unknown_bus": "无法识别该设备通过哪种总线连接，因此依赖这一点的检查没有生效。",

      "footer.diagnostics": "诊断文件：",
      "footer.by": "作者 ",
      "footer.openLog": "打开诊断文件",

      "err.state": "程序内部状态已损坏。请关闭后重新打开。",
      "err.enumerate": "无法列举设备。本程序需要管理员权限才能打开磁盘。",
      "err.device_gone": "未找到该设备；请刷新列表。",
      "err.scan_running": "已有一次检测正在进行。",
      "err.no_device": "请先选择一个设备。",
      "err.no_map": "请先检测存储卡，再进行规划。",
      "err.no_plan": "该布局已不存在；请重新计算布局。",
      "err.plan_rejected": "该布局未通过安全校验，不会被应用。原因记录在诊断文件中。",
      "err.name_mismatch": "输入的名称与设备名称不一致。",
      "err.device_blocked": "安全保护拒绝了该设备。",
      "err.open_failed": "无法打开浏览器。",
      "err.open_log": "无法打开诊断文件。",
      "err.scan.not_writable": "设备是以只读方式打开的。检测需要写入。",
      "err.scan.all_writes_failed":
        "设备拒绝了所有写入。这不是介质缺陷：很可能卷仍处于挂载状态、" +
        "卡上的写保护开关被拨到了锁定位置，或者程序没有以管理员身份运行。",
    },
  };

  /* The order the selector lists them in. */
  const ORDER = ["pt-BR", "en", "es", "zh"];

  /* Locale used for number formatting, which is not always the key. */
  const LOCALE = { "pt-BR": "pt-BR", en: "en-US", es: "es-ES", zh: "zh-CN" };

  const FALLBACK = "pt-BR";
  const STORAGE_KEY = "salvage.lang";

  /* The choice survives restarts, and it has to survive its own storage being
   * unavailable: a private window, cleared site data, a browser set to refuse
   * it. Every read and write is wrapped, and a failure just means the language
   * is picked from the system again next time. */
  function stored() {
    try {
      const v = localStorage.getItem(STORAGE_KEY);
      return v && DICT[v] ? v : null;
    } catch (_) {
      return null;
    }
  }

  function fromSystem() {
    const tags = navigator.languages && navigator.languages.length
      ? navigator.languages
      : [navigator.language || ""];
    for (const tag of tags) {
      const low = String(tag).toLowerCase();
      if (low.startsWith("pt")) return "pt-BR";
      if (low.startsWith("es")) return "es";
      if (low.startsWith("zh")) return "zh";
      if (low.startsWith("en")) return "en";
    }
    return FALLBACK;
  }

  let current = stored() || fromSystem();

  /* Substitutes {name} placeholders. Values arrive already formatted, because
   * only the caller knows whether a number is a count or a size. */
  function fill(template, params) {
    if (!params) return template;
    return template.replace(/\{(\w+)\}/g, (whole, key) =>
      Object.prototype.hasOwnProperty.call(params, key) ? String(params[key]) : whole);
  }

  /* A missing key returns the key itself, which is how a technical string from
   * a lower layer passes straight through and gets shown as-is. */
  function t(key, params) {
    const template =
      (DICT[current] && DICT[current][key]) ??
      (DICT[FALLBACK] && DICT[FALLBACK][key]) ??
      key;
    return fill(template, params);
  }

  function locale() {
    return LOCALE[current] || current;
  }

  /* Integers, grouped the way the language groups them. */
  function nf(n) {
    return Number(n).toLocaleString(locale());
  }

  /* Decimal SI, as the labels always were, but with the number formatted for
   * the language rather than for whoever wrote the backend. */
  function bytes(n) {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let v = Number(n) || 0;
    let u = 0;
    while (v >= 1000 && u < units.length - 1) {
      v /= 1000;
      u++;
    }
    const digits = u === 0 ? 0 : 2;
    return `${v.toLocaleString(locale(), {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    })} ${units[u]}`;
  }

  /* A percentage with one decimal, separator included. */
  function pct(x) {
    return Number(x).toLocaleString(locale(), {
      minimumFractionDigits: 1,
      maximumFractionDigits: 1,
    });
  }

  /* Fills every element carrying a key. `data-i18n` sets text, `data-i18n-html`
   * sets markup — the difference matters, and the attribute name is what says
   * which one a given string is allowed to be. */
  function apply(root) {
    const scope = root || document;
    scope.querySelectorAll("[data-i18n]").forEach((el) => {
      el.textContent = t(el.dataset.i18n);
    });
    scope.querySelectorAll("[data-i18n-html]").forEach((el) => {
      el.innerHTML = t(el.dataset.i18nHtml);
    });
    scope.querySelectorAll("[data-i18n-aria]").forEach((el) => {
      el.setAttribute("aria-label", t(el.dataset.i18nAria));
    });
    document.documentElement.lang = current;
  }

  function set(code) {
    if (!DICT[code]) return false;
    current = code;
    try {
      localStorage.setItem(STORAGE_KEY, code);
    } catch (_) {
      /* The language still changes for this session. */
    }
    return true;
  }

  return {
    t,
    nf,
    bytes,
    pct,
    apply,
    set,
    locale,
    order: ORDER,
    current: () => current,
    name: (code) => (DICT[code] || {})["lang.name"] || code,
  };
})();

window.I18N = I18N;
