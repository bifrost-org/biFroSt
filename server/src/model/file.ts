import { Dirent } from "fs";

// type that is similar to: https://docs.rs/fuse/latest/fuse/struct.FileAttr.html
export type FileAttr = {
  name: string;
  size: number;
  atime: string;
  mtime: string;
  ctime: string;
  crtime: string;
  kind: FileType;
  refPath?: string;
  perm: string;
  nlink: number;
};

export enum FileType {
  Directory = "directory",
  RegularFile = "regular_file",
  SymLink = "soft_link",
  HardLink = "hard_link",
}

export enum Mode {
  Write = "write",
  Append = "append",
  WriteAt = "write_at",
  Truncate = "truncate",
}

export function getNodeType(entry: Dirent): FileType {
  if (entry.isSymbolicLink()) return FileType.SymLink;
  if (entry.isDirectory()) return FileType.Directory;
  if (entry.isFile()) return FileType.RegularFile;
  throw new Error("Unknown node type");
}
