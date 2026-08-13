#!/usr/bin/env python3
"""Author descriptions for all 191 missing-description models. Factual, concise,
matching the catalog's established style. Writes patches/descriptions.json."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"

def gen():
    desc = {}

    # ---- Meta Code Llama (base/instruct/python, all sizes) ----
    for size in ("7b", "13b", "34b", "70b"):
        for variant in ("", "-instruct", "-python"):
            for hf in ("", "-hf"):
                mid = f"codellama-{size}{variant}{hf}"
                if variant == "-instruct":
                    d = (f"Code Llama {size.upper()} instruction model fine-tuned for "
                         "code generation, comprehension, and completion tasks")
                elif variant == "-python":
                    d = (f"Code Llama {size.upper()} specialized for Python code "
                         "generation and completion")
                else:
                    d = (f"Code Llama {size.upper()} base model for code completion "
                         "and infilling across popular programming languages")
                desc[mid] = d

    # ---- Google Gemma / CodeGemma ----
    desc.update({
        "gemma-2b": "Google Gemma 2B base model for efficient text generation and lightweight deployments",
        "gemma-7b": "Google Gemma 7B base model for text generation and fine-tuning",
        "gemma-2-9b-it": "Google Gemma 2 9B instruction-tuned model for chat and reasoning",
        "gemma-3-1b-it": "Google Gemma 3 1B instruction-tuned model with 32K context for efficient chat",
        "codegemma-2b": "Google CodeGemma 2B base model for code completion",
        "codegemma-7b": "Google CodeGemma 7B base model for code completion",
        "codegemma-7b-it": "Google CodeGemma 7B instruction-tuned model for code generation and chat",
        "codegemma-1.1-7b": "Google CodeGemma 1.1 7B instruction-tuned model for code generation and completion",
        "recurrentgemma-2b": "Google RecurrentGemma 2B base model with recurrent architecture for long sequences",
        "recurrentgemma-2b-it": "Google RecurrentGemma 2B instruction-tuned model for efficient long-context chat",
        "recurrentgemma-9b": "Google RecurrentGemma 9B base model with recurrent architecture for long sequences",
        "recurrentgemma-9b-it": "Google RecurrentGemma 9B instruction-tuned model for efficient long-context chat",
    })

    # ---- AI21 Jamba family ----
    desc.update({
        "jamba-1.5-large-instruct": "AI21 Jamba 1.5 Large instruction model with 256K context and hybrid SSM-transformer architecture",
        "jamba-1.5-mini-instruct": "AI21 Jamba 1.5 Mini instruction model with 256K context and hybrid SSM-transformer architecture",
        "jamba-1.6-large-instruct": "AI21 Jamba 1.6 Large instruction model with long-context hybrid architecture",
        "jamba-1.6-mini-instruct": "AI21 Jamba 1.6 Mini instruction model with long-context hybrid architecture",
        "jamba-1.7-large-instruct": "AI21 Jamba 1.7 Large instruction model with long-context hybrid architecture",
        "jamba-1.7-mini-instruct": "AI21 Jamba 1.7 Mini instruction model with long-context hybrid architecture",
        "jamba-large-1.5": "AI21 Jamba 1.5 Large instruction model (alias) with 256K context",
        "jamba-mini-1.5": "AI21 Jamba 1.5 Mini instruction model (alias) with 256K context",
        "jamba-3b-reasoning-instruct": "AI21 Jamba 3B reasoning instruction model with hybrid SSM-transformer architecture",
        "jamba-reasoning-3b": "AI21 Jamba 3B reasoning model with hybrid SSM-transformer architecture",
        "jamba-tiny-dev": "AI21 Jamba tiny development model for testing the hybrid architecture",
        "jamba-v0.1": "AI21 Jamba v0.1 base model with hybrid SSM-transformer architecture",
    })

    # ---- Mistral ----
    desc.update({
        "codestral-22b-instruct-v0.1": "Mistral Codestral 22B instruction model specialized for code generation and completion",
        "mamba-codestral-7b-v0.1": "Mistral Mamba-Codestral 7B state-space model for code generation with 32K context",
        "mistral-large-2-instruct": "Mistral Large 2 instruction model with 128K context for reasoning and agentic tasks",
        "mistral-large-instruct-2407": "Mistral Large 2 instruction model (2407 release) with 128K context",
        "mixtral-8x7b-v0.1": "Mistral Mixtral 8x7B base model with sparse mixture-of-experts architecture",
        "mixtral-8x22b-v0.1": "Mistral Mixtral 8x22B base model with sparse mixture-of-experts architecture",
        "mistral-nemo-minitron-8b-instruct": "NVIDIA Mistral NeMo Minitron 8B instruction model distilled from Mistral NeMo",
        "mistral-nemo-minitron-8b-8k-instruct": "NVIDIA Mistral NeMo Minitron 8B instruction model with 8K context",
    })

    # ---- Meta Llama 2 + guard + chatqa ----
    for size, hs in (("7b", 7), ("13b", 13), ("70b", 70)):
        for variant in ("", "-chat", "-chat-hf", "-hf"):
            mid = f"llama2-{size}{variant}"
            if "chat" in variant:
                d = f"Meta Llama 2 {hs}B chat model optimized for dialogue"
            else:
                d = f"Meta Llama 2 {hs}B base model for text generation and fine-tuning"
            desc[mid] = d
    desc.update({
        "meta-llama-guard-2-8b": "Meta Llama Guard 2 8B content-safety classifier for input/output moderation",
        "llama3-chatqa-1.5-70b": "Meta Llama 3 ChatQA 1.5 70B model fine-tuned for conversational QA and retrieval",
    })

    # ---- NVIDIA Nemotron / Nemoguard / embed / parse ----
    desc.update({
        "nemotron-4-340b-instruct": "NVIDIA Nemotron-4 340B instruction model for synthetic data generation and chat",
        "llama-3.1-nemotron-51b-instruct": "NVIDIA Llama 3.1 Nemotron 51B instruction model with 128K context",
        "llama-3.1-nemotron-ultra-253b-cpt-v1": "NVIDIA Llama 3.1 Nemotron Ultra 253B continual-pretrained model",
        "llama-3.1-nemotron-nano-vl-8b-v1-mcore": "NVIDIA Llama 3.1 Nemotron Nano 8B vision-language model",
        "llama-3.1-nemotron-nano-4b-v1.1": "NVIDIA Llama 3.1 Nemotron Nano 4B efficient instruction model",
        "llama-3.1-nemotron-8b-ultralong-1m-instruct": "NVIDIA Llama 3.1 Nemotron 8B Ultralong instruction model with 1M context",
        "llama-3.1-nemoguard-8b-content-safety": "NVIDIA Llama 3.1 Nemoguard 8B content-safety classifier",
        "llama-3.1-nemoguard-8b-topic-control": "NVIDIA Llama 3.1 Nemoguard 8B topic-control classifier",
        "nemoguard-jailbreakdetect": "NVIDIA Nemoguard jailbreak-detection classifier",
        "llama-3.2-nv-embedqa-1b-v1": "NVIDIA Llama 3.2 NV-EmbedQA 1B embedding model for retrieval",
        "llama-3.2-nv-embedqa-1b-v2": "NVIDIA Llama 3.2 NV-EmbedQA 1B v2 embedding model for retrieval",
        "llama-3.2-nemoretriever-1b-vlm-embed-v1": "NVIDIA Llama 3.2 Nemoretriever 1B VLM embedding model for multimodal retrieval",
        "llama-nemotron-embed-1b-v2": "NVIDIA Llama Nemotron Embed 1B v2 embedding model with 131K context",
        "nv-embedqa-e5-v5": "NVIDIA NV-EmbedQA E5 v5 embedding model for retrieval and RAG",
        "nemoretriever-parse": "NVIDIA Nemoretriever Parse document embedding model for retrieval",
        "nemotron-parse": "NVIDIA Nemotron Parse model for document parsing and conversion",
        "nvidia-nemotron-parse-2.0": "NVIDIA Nemotron Parse 2.0 document parsing and layout model",
        "nvidia-nemotron-parse-v1.1": "NVIDIA Nemotron Parse v1.1 document parsing model",
        "nvidia-nemotron-parse-v1.1-tc": "NVIDIA Nemotron Parse v1.1 table-content variant",
        "nvidia-nemotron-parse-v1.2": "NVIDIA Nemotron Parse v1.2 document parsing model",
        "nemotron-nano-3-30b-a3b": "NVIDIA Nemotron Nano 3 30B-A3B sparse model for efficient chat",
        "nvclip": "NVIDIA NVCLIP ViT-B-16 vision-language contrastive model for image understanding",
    })

    # ---- Writer Palmyra ----
    desc.update({
        "palmyra-creative": "Writer Palmyra Creative model for long-form creative writing with 128K context",
        "palmyra-creative-122b": "Writer Palmyra Creative 122B model for long-form creative writing",
        "palmyra-fin-70b-32k": "Writer Palmyra Fin 70B finance-specialized model with 32K context",
        "palmyra-med-70b": "Writer Palmyra Med 70B medical-specialized model with 8K context",
        "palmyra-med-70b-32k": "Writer Palmyra Med 70B medical-specialized model with 32K context",
    })

    # ---- Stockmark ----
    desc.update({
        "stockmark-2-100b-instruct": "Stockmark 2 100B Japanese instruction model for text generation",
        "stockmark-2-100b-instruct-beta": "Stockmark 2 100B Japanese instruction model (beta release)",
    })

    # ---- Zyphra Zamba2 ----
    desc.update({
        "zamba2-7b-instruct": "Zyphra Zamba2 7B instruction model with hybrid Mamba-transformer architecture",
        "zamba2-7b-instruct-v2": "Zyphra Zamba2 7B instruction model v2 with hybrid Mamba-transformer architecture",
    })

    # ---- Snowflake Arctic Embed ----
    desc.update({
        "arctic-embed-l": "Snowflake Arctic Embed L retrieval embedding model",
        "snowflake-arctic-embed-l": "Snowflake Arctic Embed L retrieval embedding model",
        "snowflake-arctic-embed-l-v2.0": "Snowflake Arctic Embed L v2 retrieval embedding model",
        "snowflake-arctic-embed-m-long": "Snowflake Arctic Embed M long-context retrieval embedding model",
    })

    # ---- Starcoder2 ----
    desc.update({
        "starcoder2-3b": "BigCode StarCoder2 3B code generation and completion model",
        "starcoder2-7b": "BigCode StarCoder2 7B code generation and completion model",
        "starcoder2-15b": "BigCode StarCoder2 15B code generation and completion model",
        "starcoder2-15b-instruct-v0.1": "BigCode StarCoder2 15B instruction model for code generation",
        "starcoder2-tokenizer": "BigCode StarCoder2 tokenizer model (no generation)",
    })

    # ---- Sea-lion ----
    desc.update({
        "sea-lion-7b-instruct": "AI Singapore SEA-LION 7B instruction model for Southeast Asian languages",
        "sea-lion-v1-7b-it": "AI Singapore SEA-LION v1 7B instruction model for Southeast Asian languages",
        "sea-lion-v1-7b-it-research": "AI Singapore SEA-LION v1 7B research instruction model",
    })

    # ---- Vision / multimodal ----
    desc.update({
        "fuyu-8b": "Adept Fuyu-8B multimodal model for image and text understanding",
        "phi-3-vision-128k-instruct": "Microsoft Phi-3 Vision 128K instruction model for image and text reasoning",
        "kosmos-2": "Microsoft Kosmos-2 multimodal model for grounded visual understanding",
        "kosmos-2-patch14-224": "Microsoft Kosmos-2 patch-14 224 model for grounded visual understanding",
        "kosmos-2.5": "Microsoft Kosmos-2.5 multimodal model for image-text understanding",
        "kosmos-2.5-chat": "Microsoft Kosmos-2.5 chat model for multimodal dialogue",
        "vila": "NVIDIA VILA vision-language model for multimodal reasoning",
        "deplot": "Google DePlot chart-to-table vision-language model",
    })

    # ---- Riva / niche ----
    desc.update({
        "riva-translate-4b-instruct": "NVIDIA Riva Translate 4B instruction model for speech translation",
        "riva-translate-4b-instruct-v2": "NVIDIA Riva Translate 4B instruction model v2 for speech translation",
        "ai-synthetic-video-detector": "Synthetic video detection model for AI-generated content classification",
        "embed-qa-4": "Embedding model for question-answering retrieval",
    })

    # ---- Granite (IBM) ----
    desc.update({
        "granite-20b-code-instruct-8k": "IBM Granite 20B code instruction model with 8K context",
        "granite-20b-code-instruct-r1.1": "IBM Granite 20B code instruction model r1.1",
        "granite-20b-functioncalling": "IBM Granite 20B function-calling model",
        "granite-3.0-1b-a400m-instruct": "IBM Granite 3.0 1B-A400M instruction model for efficient chat",
        "granite-3.0-2b-instruct": "IBM Granite 3.0 2B instruction model",
        "granite-3.0-3b-a800m-instruct": "IBM Granite 3.0 3B-A800M instruction model",
        "granite-3.0-8b-instruct": "IBM Granite 3.0 8B instruction model",
        "granite-3.0-8b-lora-intrinsics-v0.1": "IBM Granite 3.0 8B LoRA intrinsics research model",
        "granite-3.1-1b-a400m-instruct": "IBM Granite 3.1 1B-A400M instruction model",
        "granite-3.1-2b-instruct": "IBM Granite 3.1 2B instruction model",
        "granite-3.1-3b-a800m-instruct": "IBM Granite 3.1 3B-A800M instruction model",
        "granite-3.1-8b-instruct": "IBM Granite 3.1 8B instruction model with 128K context",
        "granite-3.1-8b-lora-intrinsics-v0.1": "IBM Granite 3.1 8B LoRA intrinsics research model",
        "granite-34b-code-instruct": "IBM Granite 34B code instruction model",
        "granite-34b-code-instruct-8k": "IBM Granite 34B code instruction model with 8K context",
        "granite-3b-code-instruct-128k": "IBM Granite 3B code instruction model with 128K context",
        "granite-3b-code-instruct-2k": "IBM Granite 3B code instruction model with 2K context",
        "granite-4.0-h-350m": "IBM Granite 4.0 H 350M compact instruction model",
        "granite-4.0-micro": "IBM Granite 4.0 Micro compact instruction model",
        "granite-4.1-30b": "IBM Granite 4.1 30B instruction model",
        "granite-4.1-3b": "IBM Granite 4.1 3B instruction model",
        "granite-7b-instruct": "IBM Granite 7B instruction model",
        "granite-8b-code-instruct": "IBM Granite 8B code instruction model",
        "granite-8b-code-instruct-128k": "IBM Granite 8B code instruction model with 128K context",
        "granite-8b-code-instruct-4k": "IBM Granite 8B code instruction model with 4K context",
        "granite-docling-258m": "IBM Granite Docling 258M document parsing and layout model",
        "granite-embedding-107m-multilingual": "IBM Granite Embedding 107M multilingual retrieval model",
        "granite-embedding-125m-english": "IBM Granite Embedding 125M English retrieval model",
        "granite-embedding-278m-multilingual": "IBM Granite Embedding 278M multilingual retrieval model",
        "granite-embedding-30m-english": "IBM Granite Embedding 30M English retrieval model",
        "granite-embedding-30m-sparse": "IBM Granite Embedding 30M sparse retrieval model",
        "granite-embedding-311m-multilingual-r2": "IBM Granite Embedding 311M multilingual retrieval model r2",
        "granite-embedding-97m-multilingual-r2": "IBM Granite Embedding 97M multilingual retrieval model r2",
        "granite-embedding-english-r2": "IBM Granite Embedding English retrieval model r2",
        "granite-geospatial-biomass": "IBM Granite Geospatial biomass estimation model",
        "granite-geospatial-canopyheight": "IBM Granite Geospatial canopy-height estimation model",
        "granite-geospatial-land-surface-temperature": "IBM Granite Geospatial land-surface-temperature model",
        "granite-geospatial-uki": "IBM Granite Geospatial UKI land-cover model",
        "granite-geospatial-uki-flooddetection": "IBM Granite Geospatial UKI flood-detection model",
        "granite-geospatial-wxc-downscaling": "IBM Granite Geospatial weather downscaling model",
        "granite-guardian-3.0-2b": "IBM Granite Guardian 3.0 2B safety and risk detection model",
        "granite-guardian-3.0-8b": "IBM Granite Guardian 3.0 8B safety and risk detection model",
        "granite-guardian-3.1-2b": "IBM Granite Guardian 3.1 2B safety and risk detection model",
        "granite-guardian-3.1-8b": "IBM Granite Guardian 3.1 8B safety and risk detection model",
        "granite-guardian-3.2-3b-a800m": "IBM Granite Guardian 3.2 3B-A800M safety and risk detection model",
        "granite-guardian-3.2-5b": "IBM Granite Guardian 3.2 5B safety and risk detection model",
        "granite-guardian-4.1-8b": "IBM Granite Guardian 4.1 8B safety and risk detection model",
        "granite-guardian-hap-125m": "IBM Granite Guardian HAP 125M harmful-content detection model",
        "granite-guardian-hap-38m": "IBM Granite Guardian HAP 38M harmful-content detection model",
        "granite-rag-3.0-8b-lora": "IBM Granite RAG 3.0 8B LoRA retrieval-augmented generation model",
        "granite-speech-4.1-2b": "IBM Granite Speech 4.1 2B speech synthesis model",
        "granite-speech-4.1-2b-plus": "IBM Granite Speech 4.1 2B Plus speech synthesis model",
        "granite-swash-3b-a600m": "IBM Granite Swash 3B-A600M sparse model for efficient inference",
        "granite-switch-4.1-30b-preview": "IBM Granite Switch 4.1 30B preview routing model",
        "granite-switch-4.1-3b-preview": "IBM Granite Switch 4.1 3B preview routing model",
        "granite-switch-4.1-8b-preview": "IBM Granite Switch 4.1 8B preview routing model",
        "granite-timeseries-patchtsmixer": "IBM Granite Timeseries PatchTSMixer forecasting model",
        "granite-timeseries-patchtst": "IBM Granite Timeseries PatchTST forecasting model",
        "granite-timeseries-ttm-r1": "IBM Granite Timeseries TTM r1 tiny time mixers forecasting model",
        "granite-timeseries-ttm-r2": "IBM Granite Timeseries TTM r2 tiny time mixers forecasting model",
        "granite-uncertainty-3.0-8b-lora": "IBM Granite Uncertainty 3.0 8B LoRA uncertainty-quantification model",
        "granite-vision-3.1-2b-preview": "IBM Granite Vision 3.1 2B preview vision-language model",
        "granite-vision-3.3-2b-embedding": "IBM Granite Vision 3.3 2B embedding model for multimodal retrieval",
        "granite-vision-4.1-4b": "IBM Granite Vision 4.1 4B vision-language model",
        "granitelib-rag-gpt-oss-r1.0": "IBM GraniteLib RAG GPT-OSS r1.0 retrieval model",
        "granitelib-rag-r1.0": "IBM GraniteLib RAG r1.0 retrieval model",
    })

    return desc

descs = gen()
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
# only include models actually missing description + actually in catalog
patch = {"models": {}}
for mid, d in descs.items():
    if mid not in cat:
        print(f"WARN {mid} not in catalog — skipping")
        continue
    if cat[mid].get("description"):
        print(f"WARN {mid} already has description — skipping")
        continue
    patch["models"][mid] = {"description": d}

missing = [m for m, e in cat.items() if not e.get("description")]
unfilled = [m for m in missing if m not in patch["models"]]
json.dump(patch, open(os.path.join(D, "patches", "descriptions.json"), "w"),
          ensure_ascii=False, indent=2)
print(f"\nwrote patches/descriptions.json: {len(patch['models'])} models")
print(f"still unfilled: {len(unfilled)} {unfilled[:20]}")
