import { Dirent } from "fs";

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

export function getNodeType(entry: Dirent): NodeType {
  if (entry.isSymbolicLink()) return NodeType.SoftLink;
  if (entry.isDirectory()) return NodeType.Directory;
  if (entry.isFile()) return NodeType.File;
  throw new Error("Unknown node type");
}
