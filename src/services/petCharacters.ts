import daimaoVideoUrl from "../assets/pets/daimao-batiao/daimao-batiao.mp4?url";

export type PetCharacterId = "classic" | "daimao-batiao";

export type PetCharacter = {
  id: PetCharacterId;
  name: string;
  description: string;
};

export const PET_CHARACTER_SETTING_KEY = "pet_character";

export const PET_CHARACTERS: PetCharacter[] = [
  {
    id: "classic",
    name: "默认软团",
    description: "保留当前 Canvas 互动动画",
  },
  {
    id: "daimao-batiao",
    name: "呆猫八条",
    description: "透明主体素材随机变换",
  },
];

const daimaoStillModules = import.meta.glob("../assets/pets/daimao-batiao/stills/*.png", {
  eager: true,
  import: "default",
  query: "?url",
}) as Record<string, string>;

export const DAIMAO_BATIAO_STILLS = Object.entries(daimaoStillModules)
  .sort(([left], [right]) => left.localeCompare(right))
  .map(([, url]) => url);

export const DAIMAO_BATIAO_VIDEO_URL = daimaoVideoUrl;

export function resolvePetCharacterId(value: unknown): PetCharacterId {
  return value === "daimao-batiao" ? "daimao-batiao" : "classic";
}

export function petCharacterLabel(value: PetCharacterId) {
  return PET_CHARACTERS.find((character) => character.id === value)?.name || PET_CHARACTERS[0].name;
}
