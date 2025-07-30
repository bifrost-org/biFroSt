import { Dirent, Stats } from "fs";

export type FsEntry = {
  name: string;
  type: NodeType;
  size: number;
  permissions: string;
  lastModified: string;
};

export enum NodeType {
  File = "file",
  Directory = "directory",
  SoftLink = "soft_link",
  HardLink = "hard_link",
}

export function getNodeType(entry: Dirent, stats: Stats): NodeType {
  if (entry.isFile()) return NodeType.File;
  if (entry.isDirectory()) return NodeType.Directory;
  if (entry.isSymbolicLink()) return NodeType.SoftLink;
  if (stats.nlink > 1 && stats.isFile()) return NodeType.HardLink;
  throw new Error("Unknown node type");
}
