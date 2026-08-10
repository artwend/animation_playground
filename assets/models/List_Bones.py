import os
import trimesh

def print_tree(children_map, node, prefix="", is_last=True):
    # Print the clean node name
    marker = "└── " if is_last else "├── "
    print(f"{prefix}{marker}{node}")
    
    # Calculate spacing indentations
    child_prefix = prefix + ("    " if is_last else "│   ")
    children = children_map.get(node, [])
    
    for i, child in enumerate(children):
        is_child_last = (i == len(children) - 1)
        print_tree(children_map, child, child_prefix, is_child_last)

def list_bones_from_root(glb_path, target_root="root"):
    absolute_path = os.path.abspath(glb_path)
    
    if not os.path.exists(absolute_path):
        print(f"Error: File not found at {absolute_path}")
        return

    with open(absolute_path, 'rb') as f:
        scene = trimesh.load(f, file_type='glb')
    
    if not hasattr(scene, 'graph') or not scene.graph.nodes:
        print("No node hierarchy found in this file.")
        return

    # Extract the map of {parent: [children]}
    children_map = scene.graph.transforms.children

    # Verify if your specified root node exists anywhere in the scene graph
    if target_root not in scene.graph.nodes:
        print(f"Error: A node named '{target_root}' was not found in this file.")
        print("Available nodes in this file are:")
        print(list(scene.graph.nodes))
        return

    print(f"\nSkeleton Tree starting from '{target_root}':")
    # Begin tree printing directly from the requested root node
    print_tree(children_map, target_root, prefix="", is_last=True)

# Example Usage:
list_bones_from_root('assets/models/ual1.glb', target_root="root")
